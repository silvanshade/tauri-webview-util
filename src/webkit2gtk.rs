use crate::{ApiResult, BoxError, BoxResult, Cookie, CookiePattern};
use async_stream::try_stream;
use futures::{future::BoxFuture, prelude::*, stream::BoxStream};
use tap::prelude::*;
use tauri::Window;
use url::Url;
use webkit2gtk::{gio::Cancellable, CookieManagerExt, WebContextExt, WebViewExt, WebsiteDataManagerExt};

impl crate::WebviewExt for Window {
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cache(&self) -> BoxFuture<'static, BoxResult<()>> {
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
    fn webview_delete_cookies(&self, pattern: Option<CookiePattern>) -> BoxFuture<'static, BoxResult<Vec<Cookie>>> {
        let window = self.clone();
        async move {
            let (cookie_tx, mut cookie_rx) = tokio::sync::mpsc::channel(1);
            let (result_tx, mut result_rx) = tokio::sync::mpsc::channel(1);
            let streamer = tauri::async_runtime::spawn({
                let window = window.clone();
                async move {
                    let mut cookies = vec![];
                    let mut raw_cookies = webview_get_raw_cookies(window, pattern).boxed();
                    while let Some(raw_cookie) = raw_cookies.try_next().await? {
                        cookie_tx.send(raw_cookie).await?;
                        if let Some(cookie) = result_rx.recv().await {
                            cookies.push(cookie?);
                        };
                    }
                    Ok::<_, BoxError>(cookies)
                }
            });
            let (deleter_tx, deleter_rx) = oneshot::channel();
            window.with_webview(move |webview| {
                let cookie_manager = webview
                    .inner()
                    .context()
                    .expect("failed to obtain context")
                    .cookie_manager()
                    .expect("failed to obtain cookie manager")
                    .conv::<ApiResult<_>>();
                let deleter = tauri::async_runtime::spawn(async move {
                    let callback = move |cookie: BoxResult<Cookie>| {
                        move |result: Result<(), webkit2gtk::Error>| {
                            let result = result.map_err(Into::into).and_then(|()| cookie);
                            result_tx.blocking_send(result).unwrap();
                        }
                    };
                    while let Some(mut raw_cookie) = cookie_rx.recv().await {
                        let cookie = Cookie::try_from(raw_cookie.clone());
                        let callback = callback.clone()(cookie);
                        cookie_manager.delete_cookie(&mut raw_cookie, Cancellable::current().as_ref(), callback);
                    }
                });
                deleter_tx.send(deleter).unwrap();
            })?;
            deleter_rx.await?;
            streamer.await?
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_get_cookies(&self, pattern: Option<CookiePattern>) -> BoxStream<'static, BoxResult<Cookie>> {
        let window = self.clone();
        webview_get_raw_cookies(window, pattern)
            .map(|result| result.and_then(|raw_cookie| Cookie::try_from(raw_cookie)))
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

#[cfg_attr(feature = "tracing", tracing::instrument)]
fn webview_get_raw_cookies(
    window: Window,
    pattern: Option<CookiePattern>,
) -> impl Stream<Item = BoxResult<ApiResult<soup::Cookie>>> + Send {
    let pattern = pattern.unwrap_or_default();
    let (cookie_tx, mut cookie_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = tauri::async_runtime::spawn({
        let window = window.clone();
        async move {
            let (webview_tx, webview_rx) = oneshot::channel();
            window.with_webview(move |webview| {
                let webview = webview.inner().conv::<ApiResult<_>>();
                webview_tx.send(webview).unwrap();
            })?;
            let webview = webview_rx.await?;
            let (record_tx, mut record_rx) = tokio::sync::mpsc::unbounded_channel();
            webview
                .context()
                .ok_or("failed to obtain context")?
                .website_data_manager()
                .ok_or("failed to obtain website data manager")?
                .fetch(
                    webkit2gtk::WebsiteDataTypes::COOKIES,
                    Cancellable::current().as_ref(),
                    move |result| match result {
                        Ok(data) => {
                            for record in data {
                                record_tx.send(Ok(record.conv::<ApiResult<_>>())).unwrap();
                            }
                        },
                        Err(err) => {
                            record_tx.send(Err::<_, BoxError>(err.into())).unwrap();
                        },
                    },
                );
            let callback = move |result: Result<Vec<soup::Cookie>, webkit2gtk::Error>| match result {
                Ok(cookies) => {
                    for mut cookie in cookies {
                        match pattern.cookie_matches(&mut cookie) {
                            Ok(is_match) => {
                                if is_match {
                                    let result = Ok(cookie.conv::<ApiResult<_>>());
                                    cookie_tx.send(result).unwrap();
                                }
                            },
                            Err(err) => {
                                let result = Err(err);
                                cookie_tx.send(result).unwrap();
                            },
                        }
                    }
                },
                Err(err) => {
                    cookie_tx.send(Err::<_, BoxError>(err.into())).unwrap();
                },
            };
            while let Some(record) = record_rx.recv().await {
                let domain = record?
                    .name()
                    .ok_or("failed to obtain `name` field")?
                    .trim_start_matches('.')
                    .to_string();
                for uri in [format!("https://{domain}"), format!("http://{domain}")] {
                    webview
                        .context()
                        .ok_or("failed to obtain context")?
                        .cookie_manager()
                        .ok_or("failed to obtain cookie manager")?
                        .cookies(&uri, Cancellable::current().as_ref(), callback.clone());
                }
            }
            Ok::<_, BoxError>(())
        }
    });
    try_stream! {
        while let Some(cookie) = cookie_rx.recv().await {
            yield cookie?;
        }
        handle.await??;
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

impl TryFrom<ApiResult<soup::Cookie>> for Cookie {
    type Error = <Cookie as TryFrom<soup::Cookie>>::Error;

    fn try_from(ApiResult(value): ApiResult<soup::Cookie>) -> Result<Self, Self::Error> {
        Cookie::try_from(value)
    }
}
