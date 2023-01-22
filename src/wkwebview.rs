use crate::{BoxError, BoxResult, Cookie, CookiePattern};
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
        WKWebView,
        WKWebsiteDataTypeDiskCache,
        WKWebsiteDataTypeMemoryCache,
        WKWebsiteDataTypeOfflineWebApplicationCache,
    },
};
use std::{ptr::NonNull, sync::Arc};
use tap::prelude::*;
use tauri::{window::PlatformWebview, Window};
use url::Url;

impl crate::WebViewExt for Window {
    fn webview_clear_cache(&self) -> BoxFuture<'static, BoxResult<()>> {
        let window = self.clone();
        async move {
            let notifier = tokio::sync::Notify::new().conv::<Arc<_>>();
            window.with_webview({
                let notifier = notifier.clone();
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
                    let completion_handler = ConcreteBlock::new(move || notifier.notify_one());
                    data_store.removeDataOfTypes_modifiedSince_completionHandler(
                        &data_types,
                        &date,
                        &completion_handler,
                    );
                }
            })?;
            notifier.notified().await;
            Ok(())
        }
        .boxed()
    }

    fn webview_delete_cookies(&self, pattern: CookiePattern) -> BoxFuture<'static, BoxResult<Vec<Cookie>>> {
        let window = self.clone();
        async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            window.with_webview(move |webview| unsafe {
                let webview = webview.WKWebView();
                let configuration = webview.configuration();
                let data_store = configuration.websiteDataStore();
                let http_cookie_store = data_store.httpCookieStore();
                let (get_tx, mut get_rx) = tokio::sync::mpsc::channel(1);
                let (del_tx, del_rx) = tokio::sync::mpsc::channel(1);
                http_cookie_store.getAllCookies(&ConcreteBlock::new({
                    let del_rx = tokio::sync::Mutex::new(del_rx);
                    move |array: NonNull<NSArray<NSHTTPCookie>>| {
                        for cookie in array.as_ref().iter() {
                            let result;
                            match pattern.cookie_matches(cookie) {
                                Ok(false) => continue,
                                Ok(true) => {
                                    result = Cookie::try_from(cookie);
                                    get_tx.blocking_send(cookie).unwrap();
                                    del_rx.blocking_lock().blocking_recv().unwrap();
                                },
                                Err(err) => result = Err(err),
                            }
                            tx.blocking_send(result).unwrap();
                        }
                    }
                }));
                while let Some(cookie) = get_rx.blocking_recv() {
                    http_cookie_store.deleteCookie_completionHandler(
                        cookie,
                        Some(&ConcreteBlock::new(|| {
                            del_tx.blocking_send(()).unwrap();
                        })),
                    );
                }
            })?;
            let mut cookies = vec![];
            while let Some(cookie) = rx.recv().await.transpose()? {
                cookies.push(cookie);
            }
            Ok(cookies)
        }
        .boxed()
    }

    fn webview_get_cookies(&self, pattern: CookiePattern) -> BoxResult<BoxStream<'static, BoxResult<Cookie>>> {
        let window = self.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        window.with_webview(move |webview| unsafe {
            let webview = webview.WKWebView();
            let configuration = webview.configuration();
            let data_store = configuration.websiteDataStore();
            let http_cookie_store = data_store.httpCookieStore();
            http_cookie_store.getAllCookies(&ConcreteBlock::new(|array: NonNull<NSArray<NSHTTPCookie>>| {
                for cookie in array.as_ref().iter() {
                    let result;
                    match pattern.cookie_matches(cookie) {
                        Ok(false) => continue,
                        Ok(true) => result = Cookie::try_from(cookie),
                        Err(err) => result = Err(err),
                    }
                    if tx.blocking_send(result).is_err() {
                        break;
                    }
                }
            }));
        })?;
        let stream = try_stream! {
            while let Some(cookie) = rx.recv().await.transpose()? {
                yield cookie;
            }
        }
        .boxed();
        Ok(stream)
    }

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
        })?;
        Ok(())
    }
}

trait WebViewExtForWKWebView: crate::sealed::WebViewExtForWKWebView {
    #[allow(non_snake_case)]
    unsafe fn WKWebView(&self) -> Id<WKWebView, Shared>;
}

impl crate::sealed::WebViewExtForWKWebView for PlatformWebview {
}
impl WebViewExtForWKWebView for PlatformWebview {
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
            #[cfg(feature = "time")]
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
                #[cfg(feature = "time")]
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

impl TryFrom<Id<NSHTTPCookie, Shared>> for Cookie {
    type Error = <Cookie as TryFrom<&'static NSHTTPCookie>>::Error;

    fn try_from(value: Id<NSHTTPCookie, Shared>) -> Result<Self, Self::Error> {
        Cookie::try_from(&*value)
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

impl CookiePattern {
    pub(crate) fn cookie_matches(&self, cookie: &NSHTTPCookie) -> BoxResult<bool> {
        let domain = unsafe { cookie.domain() }.to_string();
        let domain = domain.trim_start_matches('.');
        let secure = unsafe { cookie.isSecure() };
        Ok((self.matcher)(domain, secure))
    }
}
