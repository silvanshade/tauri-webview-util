use crate::{ApiResult, BoxError, BoxResult, Cookie};
use futures::{future::BoxFuture, prelude::*};
use tauri::Window;
use url::Url;
use webkit2gtk::{gio::Cancellable, CookieManager, CookieManagerExt, WebContextExt, WebViewExt, WebsiteDataManagerExt};

impl crate::WebviewExt for Window {
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cache(&self) -> BoxFuture<BoxResult<()>> {
        let window = self.clone();
        async move {
            let (done_tx, done_rx) = oneshot::channel();
            window.with_webview(move |webview| {
                let webview = webview.inner();
                if let Some(context) = webview.context() {
                    context.clear_cache();
                }
                done_tx.send(()).unwrap();
            })?;
            done_rx.await?;
            Ok(())
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_delete_cookies(&self, url: Option<Url>) -> BoxFuture<BoxResult<Vec<Cookie>>> {
        async move {
            let mut cookies = vec![];
            if let Some(cookie_manager) = webview_get_cookie_manager(self).await? {
                let raw_cookies = webview_get_raw_cookies(self, url).await?;
                let raw_cookies = raw_cookies.lock()?;
                let cookie_manager = cookie_manager.lock()?;
                for mut raw_cookie in raw_cookies.iter().cloned() {
                    let cancellable = Cancellable::current();
                    let (done_tx, done_rx) = oneshot::channel();
                    cookie_manager.delete_cookie(&mut raw_cookie, cancellable.as_ref(), |result| {
                        done_tx.send(result).unwrap();
                    });
                    done_rx.recv()??;
                    cookies.push(raw_cookie.try_into()?);
                }
            }
            Ok(cookies)
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_get_cookies(&self, url: Option<Url>) -> BoxFuture<BoxResult<Vec<Cookie>>> {
        async move {
            let cookies = webview_get_raw_cookies(self, url)
                .await?
                .lock()?
                .iter()
                .cloned()
                .map(TryInto::try_into)
                .collect::<BoxResult<Vec<_>>>()?;
            Ok(cookies)
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_navigate(&self, url: Url) -> BoxResult<()> {
        self.with_webview(move |webview| {
            let webview = webview.inner();
            webview.load_uri(url.as_str());
        })?;
        Ok(())
    }
}

impl TryFrom<soup::Cookie> for Cookie {
    type Error = BoxError;

    fn try_from(mut cookie: soup::Cookie) -> Result<Self, Self::Error> {
        fn unexpectedly_null(field: &str) -> BoxError {
            format!("field `{field}` unexpectedly null").into()
        }
        let name = cookie.name().map(Into::into).ok_or(unexpectedly_null("name"))?;
        let value = cookie.value().map(Into::into).ok_or(unexpectedly_null("value"))?;
        let domain = cookie.domain().map(Into::into).ok_or(unexpectedly_null("domain"))?;
        let path = cookie.path().map(Into::into).ok_or(unexpectedly_null("path"))?;
        let port_list = None;
        let expires = cookie
            .expires()
            .and_then(|mut date| {
                let format = soup::DateFormat::Iso8601Full;
                date.to_string(format).map(Into::<String>::into)
            })
            .map(|s| {
                let description = time::format_description::well_known::Iso8601::PARSING;
                time::OffsetDateTime::parse(&s, &description)
            })
            .transpose()?;
        let is_http_only = cookie.is_http_only();
        let same_site = None;
        let is_secure = cookie.is_secure();
        let is_session = None;
        let comment = None;
        let comment_url = None;
        Ok(Self {
            name,
            value,
            domain,
            path,
            port_list,
            expires,
            is_http_only,
            same_site,
            is_secure,
            is_session,
            comment,
            comment_url,
        })
    }
}

#[cfg_attr(feature = "tracing", tracing::instrument)]
async fn webview_get_cookie_manager(window: &Window) -> BoxResult<Option<ApiResult<CookieManager>>> {
    let (call_tx, call_rx) = oneshot::channel();
    window.with_webview(move |webview| {
        let webview = webview.inner();
        if let Some(context) = webview.context() {
            let cookie_manager = context.cookie_manager().map(ApiResult::new);
            call_tx.send(cookie_manager).unwrap();
        }
    })?;
    Ok(call_rx.await?)
}

#[cfg_attr(feature = "tracing", tracing::instrument)]
async fn webview_get_raw_cookies(window: &Window, url: Option<Url>) -> BoxResult<ApiResult<Vec<soup::Cookie>>> {
    if let Some(url) = url {
        webview_get_raw_cookies_for_one_urls(window, url).await.map(Into::into)
    } else {
        webview_get_raw_cookies_for_all_urls(window).await
    }
}

#[cfg_attr(feature = "tracing", tracing::instrument)]
async fn webview_get_raw_cookies_for_one_urls(window: &Window, url: Url) -> BoxResult<Vec<soup::Cookie>> {
    let (call_tx, call_rx) = oneshot::channel::<ApiResult<_>>();
    window.with_webview(move |webview| {
        let webview = webview.inner();
        if let Some(context) = webview.context() {
            if let Some(cookie_manager) = context.cookie_manager() {
                let url = url.as_str();
                let cancellable = Cancellable::current();
                // NOTE: this function appears to not return cookies for some domains reported as
                // having cookies by either the data manager or the deprecated cookie manager
                // function that reports all domains with cookies. It's unclear if this is a bug in
                // webkit2gtk or if something else is going on. Currently this means that getting
                // all cookies with web2gtk is unreliable compared to the other platforms.
                cookie_manager.cookies(url, cancellable.as_ref(), |result| {
                    call_tx.send(result.into()).unwrap();
                });
            }
        }
    })?;
    Ok(call_rx.await?.lock()?.clone()?)
}

#[cfg_attr(feature = "tracing", tracing::instrument)]
async fn webview_get_raw_cookies_for_all_urls(window: &Window) -> BoxResult<ApiResult<Vec<soup::Cookie>>> {
    use itertools::Itertools;
    let urls = webview_get_all_domains_with_cookies(window)
        .await?
        .iter()
        .map(|name| {
            let http = Url::parse(&format!("http://{}", name))?;
            let https = Url::parse(&format!("https://{}", name))?;
            Ok::<_, BoxError>(vec![http, https])
        })
        .flatten_ok()
        .collect::<BoxResult<Vec<_>>>()?;
    let cookies = ApiResult::new(vec![]);
    for url in urls {
        let data = &mut webview_get_raw_cookies_for_one_urls(window, url).await?;
        cookies.lock()?.append(data);
    }
    Ok(cookies)
}

async fn webview_get_all_domains_with_cookies(window: &Window) -> BoxResult<Vec<String>> {
    let (call_tx, call_rx) = oneshot::channel::<ApiResult<_>>();
    window.with_webview(move |webview| {
        let webview = webview.inner();
        if let Some(context) = webview.context() {
            if let Some(website_data_manager) = context.website_data_manager() {
                let types = webkit2gtk::WebsiteDataTypes::COOKIES;
                let cancellable = Cancellable::current();
                website_data_manager.fetch(types, cancellable.as_ref(), |result| {
                    call_tx.send(result.into()).unwrap();
                })
            }
        }
    })?;
    let domains = call_rx.await?;
    let domains = match &*domains.lock()? {
        Ok(domains) => domains
            .iter()
            .filter_map(|domain| domain.name().map(Into::into))
            .collect(),
        Err(err) => return Err(err.clone().into()),
    };
    Ok(domains)
}
