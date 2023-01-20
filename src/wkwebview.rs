use crate::{ApiResult, BoxError, BoxResult, Cookie, CookiePattern};
use async_stream::try_stream;
use block2::ConcreteBlock;
use futures::{future::BoxFuture, prelude::*, stream::BoxStream};
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
use std::{collections::HashSet, ptr::NonNull, sync::Arc};
use tap::prelude::*;
use tauri::{window::PlatformWebview, Window};
use url::Url;

impl crate::WebviewExt for Window {
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cache(&self) -> BoxFuture<'static, BoxResult<()>> {
        let window = self.clone();
        async move {
            let done = dispatch::Semaphore::new(0);
            window
                .with_webview({
                    let done = done.clone();
                    |webview| unsafe {
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
    fn webview_delete_cookies(&self, pattern: Option<CookiePattern>) -> BoxFuture<'static, BoxResult<Vec<Cookie>>> {
        let window = self.clone();
        async move {
            let (cookie_tx, mut cookie_rx) = tokio::sync::mpsc::channel(1);
            let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<BoxResult<Cookie>>(1);
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
            window.with_webview(move |webview| unsafe {
                let webview = webview.WKWebView();
                let configuration = webview.configuration();
                let data_store = configuration.websiteDataStore();
                let http_cookie_store = data_store.httpCookieStore().conv::<ApiResult<_>>();
                let deleter = tauri::async_runtime::spawn(async move {
                    while let Some(raw_cookie) = cookie_rx.recv().await {
                        tokio::task::block_in_place(|| {
                            let raw_cookie = &**raw_cookie.blocking_lock();
                            let done = dispatch::Semaphore::new(0);
                            let completion_handler = ConcreteBlock::new({
                                let done = done.clone();
                                move || {
                                    done.signal();
                                }
                            })
                            .copy();
                            http_cookie_store
                                .blocking_lock()
                                .deleteCookie_completionHandler(raw_cookie, Some(&completion_handler));
                            result_tx.blocking_send(Cookie::try_from(raw_cookie))?;
                            done.wait();
                            Ok::<_, BoxError>(())
                        })?;
                    }
                    Ok::<_, BoxError>(())
                });
                deleter_tx.send(deleter).unwrap();
            })?;
            deleter_rx.await?.await??;
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

#[cfg_attr(feature = "tracing", tracing::instrument)]
fn webview_get_raw_cookies(
    window: Window,
    pattern: Option<CookiePattern>,
) -> impl Stream<Item = BoxResult<ApiResult<Id<NSHTTPCookie, Shared>>>> + Send {
    let pattern = pattern.unwrap_or_default();
    let (cookie_tx, mut cookie_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = tauri::async_runtime::spawn({
        let window = window.clone();
        async move {
            let done = dispatch::Semaphore::new(0);
            window.with_webview({
                let done = done.clone();
                |webview| unsafe {
                    let webview = webview.WKWebView();
                    let configuration = webview.configuration();
                    let data_store = configuration.websiteDataStore();
                    let http_cookie_store = data_store.httpCookieStore();
                    http_cookie_store.getAllCookies(
                        &ConcreteBlock::new(move |array: NonNull<NSArray<NSHTTPCookie>>| {
                            for cookie in array.as_ref().to_shared_vec() {
                                match pattern.cookie_matches(&cookie) {
                                    Ok(is_match) => {
                                        if is_match {
                                            let result = Ok(cookie.conv::<ApiResult<_>>());
                                            if cookie_tx.send(result).is_err() {
                                                break;
                                            }
                                        }
                                    },
                                    Err(err) => {
                                        let result = Err(err);
                                        if cookie_tx.send(result).is_err() {
                                            break;
                                        }
                                    },
                                }
                            }
                            done.signal();
                        })
                        .copy(),
                    );
                }
            })?;
            done.future().await?;
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

impl TryFrom<&NSHTTPCookie> for Cookie {
    type Error = BoxError;

    fn try_from(cookie: &NSHTTPCookie) -> Result<Self, Self::Error> {
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
            let is_http_only = cookie.isHTTPOnly().into();
            let same_site = cookie.sameSitePolicy().map(|policy| policy.to_string());
            let is_secure = cookie.isSecure().into();
            let is_session = cookie.isSessionOnly().into();
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
                is_http_only,
                same_site,
                is_secure,
                is_session,
                comment,
                comment_url,
            })
        }
    }
}

impl TryFrom<ApiResult<Id<NSHTTPCookie, Shared>>> for Cookie {
    type Error = <Cookie as TryFrom<&'static NSHTTPCookie>>::Error;

    fn try_from(value: ApiResult<Id<NSHTTPCookie, Shared>>) -> Result<Self, Self::Error> {
        let cookie = Arc::try_unwrap(value.0)
            .map_err(|_| "failed to unwrap Arc")?
            .into_inner();
        Cookie::try_from(&*cookie)
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
        async {
            if icrate::Foundation::is_main_thread() {
                let this = self.clone();
                tokio::task::spawn_blocking(move || this.wait()).await?;
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
