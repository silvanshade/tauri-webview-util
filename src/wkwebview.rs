use crate::{ApiResult, BoxError, BoxResult, Cookie};
use block2::ConcreteBlock;
use futures::{future::BoxFuture, prelude::*};
use icrate::{
    objc2::{
        rc::{Id, Shared},
        *,
    },
    Foundation::{NSArray, NSDate, NSHTTPCookie, NSNumber, NSSet, NSString, NSURLRequest, NSURL},
    WebKit::{
        WKHTTPCookieStore,
        WKWebView,
        WKWebsiteDataTypeDiskCache,
        WKWebsiteDataTypeMemoryCache,
        WKWebsiteDataTypeOfflineWebApplicationCache,
    },
};
use std::{collections::HashSet, ptr::NonNull};
use tauri::{window::PlatformWebview, Window};
use url::Url;

impl crate::WebviewExt for Window {
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cache(&self) -> BoxFuture<BoxResult<()>> {
        let window = self.clone();
        async move {
            let done = dispatch::Semaphore::new(0);
            window
                .with_webview({
                    let done = done.clone();
                    move |webview| unsafe {
                        let webview = webview.WKWebView();
                        let configuration = webview.configuration();
                        let data_store = configuration.websiteDataStore();
                        let data_types = NSSet::from_slice(&[
                            WKWebsiteDataTypeMemoryCache.to_owned(),
                            WKWebsiteDataTypeDiskCache.to_owned(),
                            WKWebsiteDataTypeOfflineWebApplicationCache.to_owned(),
                        ]);
                        let date = NSDate::distantPast();
                        let completion_handler = ConcreteBlock::new(move || {
                            done.signal();
                        })
                        .copy();
                        data_store.removeDataOfTypes_modifiedSince_completionHandler(
                            &data_types,
                            &date,
                            &completion_handler,
                        );
                    }
                })
                .map_err(Into::<BoxError>::into)?;
            done.future().await?;
            Ok(())
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_delete_cookies(&self, url: Option<Url>) -> BoxFuture<BoxResult<Vec<Cookie>>> {
        async move {
            let mut result = vec![];
            let cookie_manager = webview_get_cookie_manager(self).await?;
            let cookies = {
                let iter = webview_get_raw_cookies(self, url.as_ref()).await?;
                iter.map(ApiResult::new).collect::<Vec<_>>()
            };
            for cookie in cookies {
                let done = dispatch::Semaphore::new(0);
                let (done_tx, done_rx) = oneshot::channel();
                self.run_on_main_thread({
                    let manager = cookie_manager.clone();
                    let done = done.clone();
                    move || {
                        let manager = manager.lock().unwrap();
                        let cookie = cookie.lock().unwrap();
                        let _: () = unsafe {
                            manager.deleteCookie_completionHandler(
                                &cookie,
                                Some(
                                    &ConcreteBlock::new(move || {
                                        done.signal();
                                    })
                                    .copy(),
                                ),
                            )
                        };
                        done_tx.send((&*cookie).try_into()).unwrap();
                    }
                })?;
                done.future().await?;
                result.push(done_rx.recv()??);
            }
            Ok(result)
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_get_cookies(&self, url: Option<Url>) -> BoxFuture<BoxResult<Vec<Cookie>>> {
        async move {
            webview_get_raw_cookies(self, url.as_ref())
                .await?
                .map(|cookie| Cookie::try_from(&cookie))
                .collect::<BoxResult<Vec<_>>>()
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_navigate(&self, url: Url) -> BoxResult<()> {
        self.with_webview(move |webview| unsafe {
            let webview = webview.WKWebView();
            let string = NSString::from_str(url.as_str());
            if let Some(url) = NSURL::URLWithString(&string) {
                let request = NSURLRequest::requestWithURL(&url);
                #[allow(unused_variables)]
                webview.loadRequest(&request);
                #[cfg(feature = "tracing")]
                tracing::info!(?navigation);
            }
        })
        .map_err(Into::into)
    }
}

impl TryFrom<&Id<NSHTTPCookie, Shared>> for Cookie {
    type Error = BoxError;

    fn try_from(cookie: &Id<NSHTTPCookie, Shared>) -> Result<Self, Self::Error> {
        unsafe {
            let name = cookie.name().to_string().into();
            let value = cookie.value().to_string().into();
            let domain = cookie.domain().to_string().into();
            let path = cookie.path().to_string().into();
            let port_list = cookie
                .portList()
                .map(|list| list.into_iter().map(|port| u16::try_from(Number::from(port))).collect())
                .transpose()?;
            let expires = cookie
                .expiresDate()
                .map(|date| {
                    let timestamp = date.timeIntervalSince1970().round() as i64;
                    time::OffsetDateTime::from_unix_timestamp(timestamp)
                })
                .transpose()?;
            let http_only = cookie.isHTTPOnly().into();
            let same_site = cookie.sameSitePolicy().map(|policy| policy.to_string());
            let secure = cookie.isSecure().into();
            let session = cookie.isSessionOnly().into();
            let comment = cookie.comment().map(|comment| comment.to_string());
            let comment_url = cookie
                .commentURL()
                .and_then(|url| url.absoluteString().map(|url| Url::parse(&url.to_string())))
                .transpose()?;
            Ok(Self {
                name,
                value,
                domain,
                path,
                port_list,
                expires,
                http_only,
                same_site,
                secure,
                session,
                comment,
                comment_url,
            })
        }
    }
}

enum Number {
    Signed(i64),
    Unsigned(u64),
    Floating(f64),
}

impl From<&NSNumber> for Number {
    fn from(n: &NSNumber) -> Self {
        match n.encoding() {
            Encoding::Char | Encoding::Short | Encoding::Int | Encoding::Long | Encoding::LongLong => {
                Self::Signed(n.as_i64())
            },
            Encoding::UChar | Encoding::UShort | Encoding::UInt | Encoding::ULong | Encoding::ULongLong => {
                Self::Unsigned(n.as_u64())
            },
            Encoding::Float | Encoding::Double => Self::Floating(n.as_f64()),
            _ => unreachable!(),
        }
    }
}

impl TryFrom<Number> for u16 {
    type Error = crate::BoxError;

    fn try_from(number: Number) -> Result<Self, Self::Error> {
        let value = match number {
            Number::Signed(i) => u16::try_from(i)?,
            Number::Unsigned(u) => u16::try_from(u)?,
            Number::Floating(f) => u16::try_from(f.round() as i64)?,
        };
        Ok(value)
    }
}

#[cfg_attr(feature = "tracing", tracing::instrument)]
async fn webview_get_cookie_manager(window: &Window) -> BoxResult<ApiResult<Id<WKHTTPCookieStore, Shared>>> {
    let (call_tx, call_rx) = oneshot::channel::<ApiResult<_>>();
    window.with_webview(move |webview| unsafe {
        let webview = webview.WKWebView();
        let configuration = webview.configuration();
        let data_store = configuration.websiteDataStore();
        let http_cookie_store = data_store.httpCookieStore();
        call_tx.send(http_cookie_store.into()).unwrap();
    })?;
    Ok(call_rx.await?)
}

#[cfg_attr(feature = "tracing", tracing::instrument)]
async fn webview_get_raw_cookies<'a>(
    window: &Window,
    url: Option<&'a Url>,
) -> BoxResult<impl Iterator<Item = Id<NSHTTPCookie, Shared>> + 'a> {
    let filter = webview_cookie_filter(url)?;
    let cookies = {
        let iter = webview_get_raw_cookies_for_all_domains(window).await?;
        iter.filter(move |cookie| unsafe {
            let domain = cookie.domain().to_string();
            let secure = cookie.isSecure();
            filter(domain, secure)
        })
    };
    Ok(cookies)
}

#[cfg_attr(feature = "tracing", tracing::instrument)]
async fn webview_get_raw_cookies_for_all_domains(
    window: &Window,
) -> BoxResult<impl Iterator<Item = Id<NSHTTPCookie, Shared>>> {
    let done = dispatch::Semaphore::new(0);
    let done_val = ApiResult::new(Vec::new());
    window.with_webview({
        let done = done.clone();
        let done_val = done_val.clone();
        move |webview| unsafe {
            let webview = webview.WKWebView();
            let configuration = webview.configuration();
            let data_store = configuration.websiteDataStore();
            let http_cookie_store = data_store.httpCookieStore();
            http_cookie_store.getAllCookies(
                &*ConcreteBlock::new(move |array: NonNull<NSArray<NSHTTPCookie>>| {
                    *done_val.lock().unwrap() = array.as_ref().to_shared_vec();
                    done.signal();
                })
                .copy(),
            );
        }
    })?;
    done.future().await?;
    let mut cookies = HashSet::new();
    for cookie in done_val.lock()?.iter() {
        cookies.insert(cookie.clone().try_into()?);
    }
    Ok(cookies.into_iter())
}

#[cfg_attr(feature = "tracing", tracing::instrument)]
fn webview_cookie_filter<'a>(url: Option<&'a Url>) -> BoxResult<impl Fn(String, bool) -> bool + Send + Sync + 'a> {
    fn identity<'a>() -> Box<dyn Fn(String, bool) -> bool + Send + Sync + 'a> {
        Box::new(|_domain, _secure| true)
    }
    fn with_url_and_host<'a>(url: &'a Url, host: String) -> Box<dyn Fn(String, bool) -> bool + Send + Sync + 'a> {
        Box::new(move |domain, secure| {
            domain
                .strip_suffix(&host)
                .map(|prefix| prefix == "" || prefix.ends_with('.'))
                .unwrap_or_default()
                && if url.scheme() == "https" { secure } else { !secure }
        })
    }
    match url {
        None => Ok(identity()),
        Some(url) => match url.host_str().map(Into::<String>::into) {
            None => {
                let msg = format!(r#""{url}" has no host"#);
                return Err(msg.into());
            },
            Some(host) => Ok(with_url_and_host(url, host)),
        },
    }
}

trait WebviewExtForWKWebView: private::WebviewExtForWKWebViewSealed {
    #[allow(non_snake_case)]
    unsafe fn WKWebView(&self) -> Id<WKWebView, Shared>;
}

impl WebviewExtForWKWebView for PlatformWebview {
    #[allow(non_snake_case)]
    unsafe fn WKWebView(&self) -> Id<WKWebView, Shared> {
        let src = self.inner();
        let ptr = std::mem::transmute::<_, *mut WKWebView>(src);
        match Id::retain_autoreleased(ptr) {
            None => unreachable!("pointer should never be null"),
            Some(wkwebview) => wkwebview,
        }
    }
}

trait SemaphoreExt: private::SemaphoreExtSealed {
    fn future(&self) -> BoxFuture<BoxResult<()>>;
}

impl SemaphoreExt for dispatch::Semaphore {
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn future(&self) -> BoxFuture<BoxResult<()>> {
        async move {
            if icrate::Foundation::is_main_thread() {
                let this = self.clone();
                tokio::task::spawn_blocking(move || {
                    this.wait();
                })
                .await?;
            } else {
                self.wait();
            }
            Ok(())
        }
        .boxed()
    }
}

mod private {
    pub trait SemaphoreExtSealed {}
    impl SemaphoreExtSealed for dispatch::Semaphore {
    }

    pub trait WebviewExtForWKWebViewSealed {}
    impl WebviewExtForWKWebViewSealed for tauri::window::PlatformWebview {
    }
}
