use futures::{future::BoxFuture, prelude::*};

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
use webkit2gtk::{WebContextExt, WebViewExt, WebsiteDataManagerExtManual, WebsiteDataTypes};

#[cfg(target_os = "macos")]
use ::{
    block::ConcreteBlock,
    cocoa::{
        base::{id, nil},
        foundation::{NSArray, NSString},
    },
    objc::*,
};

#[cfg(target_os = "windows")]
use ::{
    tauri::window::PlatformWebview,
    webview2_com::{
        CallDevToolsProtocolMethodCompletedHandler,
        Error::WindowsError,
        Microsoft::Web::WebView2::Win32::{ICoreWebView2Cookie, COREWEBVIEW2_COOKIE_SAME_SITE_KIND},
    },
    windows::{
        core::{HSTRING, PCWSTR, PWSTR},
        w,
        Win32::Foundation::BOOL,
    },
};

#[derive(Clone, Debug)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: f64,
    pub is_http_only: bool,
    pub same_site: i32,
    pub is_secure: bool,
    pub is_session: bool,
}

#[cfg(target_os = "windows")]
impl TryFrom<ICoreWebView2Cookie> for Cookie {
    type Error = BoxError;

    fn try_from(cookie: ICoreWebView2Cookie) -> Result<Self, Self::Error> {
        let name = &mut PWSTR::null();
        let value = &mut PWSTR::null();
        let domain = &mut PWSTR::null();
        let path = &mut PWSTR::null();
        let expires = &mut f64::default();
        let is_http_only = &mut BOOL::default();
        let same_site = &mut COREWEBVIEW2_COOKIE_SAME_SITE_KIND::default();
        let is_secure = &mut BOOL::default();
        let is_session = &mut BOOL::default();

        unsafe {
            cookie.Name(name)?;
            cookie.Value(value)?;
            cookie.Domain(domain)?;
            cookie.Path(path)?;
            cookie.Expires(expires)?;
            cookie.IsHttpOnly(is_http_only)?;
            cookie.SameSite(same_site)?;
            cookie.IsSecure(is_secure)?;
            cookie.IsSession(is_session)?;

            let name = name.to_string()?;
            let value = value.to_string()?;
            let domain = domain.to_string()?;
            let path = path.to_string()?;
            let expires = *expires;
            let is_http_only = is_http_only.as_bool();
            let same_site = same_site.0;
            let is_secure = is_secure.as_bool();
            let is_session = is_session.as_bool();

            Ok(Self {
                name,
                value,
                domain,
                path,
                expires,
                is_http_only,
                same_site,
                is_secure,
                is_session,
            })
        }
    }
}

pub trait WebviewExt: private::Sealed {
    fn webview_clear_cache(&self) -> BoxFuture<Result<(), BoxError>>;
    fn webview_clear_cookies(&self) -> BoxFuture<Result<(), BoxError>>;
    fn webview_get_all_cookies(&self) -> BoxFuture<Result<Option<Vec<Cookie>>, BoxError>>;
    fn webview_navigate(&self, url: url::Url) -> Result<(), BoxError>;
}

#[cfg(target_os = "macos")]
impl WebviewExt for tauri::Window {
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cache(&self) -> BoxFuture<Result<(), BoxError>> {
        let window = self.clone();
        async move {
            let semaphore = dispatch::Semaphore::new(0);
            window
                .with_webview({
                    let semaphore = semaphore.clone();
                    move |webview| unsafe {
                        let webview = webview.inner();
                        let configuration: id = msg_send![webview, configuration];
                        let data_store: id = msg_send![configuration, websiteDataStore];
                        let data_types: id = msg_send![class!(NSSet), setWithArray: NSArray::arrayWithObjects(nil, &[
                            NSString::alloc(nil).init_str("WKWebsiteDataTypeMemoryCache"),
                            NSString::alloc(nil).init_str("WKWebsiteDataTypeDiskCache"),
                            NSString::alloc(nil).init_str("WKWebsiteDataTypeOfflineWebApplicationCache"),
                        ])];
                        let date: id = msg_send![class!(NSDate), distantPast];
                        let handler = ConcreteBlock::new(move || semaphore.signal()).copy();
                        let _: () = msg_send![data_store, removeDataOfTypes: data_types modifiedSince: date completionHandler: handler];
                    }
                })
                .map_err(Into::<BoxError>::into)?;
            semaphore.wait();
            Ok(())
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cookies(&self) -> BoxFuture<Result<(), BoxError>> {
        let window = self.clone();
        async move {
            let semaphore = dispatch::Semaphore::new(0);
            window
                .with_webview({
                    let semaphore = semaphore.clone();
                    move |webview| unsafe {
                        let webview = webview.inner();
                        let configuration: id = msg_send![webview, configuration];
                        let data_store: id = msg_send![configuration, websiteDataStore];
                        let data_types: id = msg_send![class!(NSSet), setWithArray: NSArray::arrayWithObjects(nil, &[
                            NSString::alloc(nil).init_str("WKWebsiteDataTypeCookies"),
                        ])];
                        let date: id = msg_send![class!(NSDate), distantPast];
                        let handler = ConcreteBlock::new(move || semaphore.signal()).copy();
                        let _: () = msg_send![data_store, removeDataOfTypes: data_types modifiedSince: date completionHandler: handler];
                    }
                })
                .map_err(Into::<BoxError>::into)?;
            semaphore.wait();
            Ok(())
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_navigate(&self, url: url::Url) -> Result<(), BoxError> {
        self.with_webview(move |webview| unsafe {
            let webview = webview.inner();
            let string = NSString::alloc(nil).init_str(url.as_str());
            let url: id = msg_send![class!(NSURL), URLWithString: string];
            let request: id = msg_send![class!(NSURLRequest), requestWithURL: url];
            #[allow(unused_variables)]
            let navigation: id = msg_send![webview, loadRequest: request];
            #[cfg(feature = "tracing")]
            tracing::info!(?navigation);
        })
        .map_err(Into::into)
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
impl WebviewExt for tauri::Window {
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cache(&self) -> BoxFuture<Result<(), BoxError>> {
        let window = self.clone();
        async move {
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            window
                .with_webview(move |webview| {
                    let webview = webview.inner();
                    if let Some(context) = webview.context() {
                        context.clear_cache();
                    }
                    done_tx.send(()).unwrap();
                })
                .map_err(Into::<BoxError>::into)?;
            done_rx.await?;
            Ok(())
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cookies(&self) -> BoxFuture<Result<(), BoxError>> {
        let window = self.clone();
        async move {
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            window
                .with_webview(move |webview| {
                    let webview = webview.inner();
                    if let Some(manager) = webview.website_data_manager() {
                        let types = WebsiteDataTypes::DISK_CACHE
                            | WebsiteDataTypes::MEMORY_CACHE
                            | WebsiteDataTypes::OFFLINE_APPLICATION_CACHE;
                        let timespan = webkit2gtk::glib::TimeSpan::from_seconds(0);
                        let cancellable = webkit2gtk::gio::Cancellable::current();
                        manager.clear(types, timespan, cancellable.as_ref(), |result| {
                            result.unwrap();
                            done_tx.send(()).unwrap();
                        });
                    }
                })
                .map_err(Into::<BoxError>::into)?;
            done_rx.await?;
            Ok(())
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_navigate(&self, url: url::Url) -> Result<(), BoxError> {
        self.with_webview(move |webview| {
            let webview = webview.inner();
            webview.load_uri(url.as_str());
        })
        .map_err(Into::into)
    }
}

#[cfg(target_os = "windows")]
impl WebviewExt for tauri::Window {
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cache(&self) -> BoxFuture<Result<(), BoxError>> {
        unsafe fn run(webview: PlatformWebview, done_tx: tokio::sync::oneshot::Sender<()>) -> Result<(), wry::Error> {
            let webview = webview.controller().CoreWebView2().map_err(WindowsError)?;
            CallDevToolsProtocolMethodCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    webview.CallDevToolsProtocolMethod(w!("Network.clearBrowserCache"), w!("{}"), &handler)?;
                    Ok(())
                }),
                #[allow(unused_variables)]
                Box::new(move |hresult, pcwstr| {
                    hresult?;
                    #[cfg(feature = "tracing")]
                    tracing::info!(?pcwstr);
                    done_tx.send(()).unwrap();
                    Ok(())
                }),
            )?;
            Ok(())
        }

        let window = self.clone();
        async move {
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            let (call_tx, call_rx) = tokio::sync::oneshot::channel();
            window
                .with_webview(move |webview| unsafe {
                    let result = run(webview, done_tx).map_err(Into::into);
                    call_tx.send(result).unwrap();
                })
                .map_err(Into::<BoxError>::into)
                .and(call_rx.await?)?;
            done_rx.await?;
            Ok(())
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cookies(&self) -> BoxFuture<Result<(), BoxError>> {
        unsafe fn run(webview: PlatformWebview, done_tx: tokio::sync::oneshot::Sender<()>) -> Result<(), wry::Error> {
            let webview = webview.controller().CoreWebView2().map_err(WindowsError)?;
            CallDevToolsProtocolMethodCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    webview.CallDevToolsProtocolMethod(w!("Network.clearBrowserCookies"), w!("{}"), &handler)?;
                    Ok(())
                }),
                #[allow(unused_variables)]
                Box::new(move |hresult, pcwstr| {
                    hresult?;
                    #[cfg(feature = "tracing")]
                    tracing::info!(?pcwstr);
                    done_tx.send(()).unwrap();
                    Ok(())
                }),
            )?;
            Ok(())
        }

        let window = self.clone();
        async move {
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            let (call_tx, call_rx) = tokio::sync::oneshot::channel();
            window
                .with_webview(move |webview| unsafe {
                    let result = run(webview, done_tx).map_err(Into::into);
                    call_tx.send(result).unwrap();
                })
                .map_err(Into::<BoxError>::into)
                .and(call_rx.await?)?;
            done_rx.await?;
            Ok(())
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_get_all_cookies(&self) -> BoxFuture<Result<Option<Vec<Cookie>>, BoxError>> {
        use std::sync::{Arc, Mutex};
        use webview2_com::{
            GetCookiesCompletedHandler,
            Microsoft::Web::WebView2::Win32::{ICoreWebView2CookieList, ICoreWebView2_15},
        };

        #[derive(Debug)]
        struct GetCookiesResult(Arc<Mutex<ICoreWebView2CookieList>>);

        impl GetCookiesResult {
            fn new(list: ICoreWebView2CookieList) -> Self {
                Self(Arc::new(Mutex::new(list)))
            }

            fn inner(&self) -> &Arc<Mutex<ICoreWebView2CookieList>> {
                &self.0
            }
        }

        unsafe impl Send for GetCookiesResult {
        }
        unsafe impl Sync for GetCookiesResult {
        }

        unsafe fn run(
            webview: PlatformWebview,
            done_tx: tokio::sync::oneshot::Sender<Option<GetCookiesResult>>,
        ) -> Result<(), wry::Error> {
            let webview = webview.controller().CoreWebView2().map_err(WindowsError)?;
            let webview = windows::core::Interface::cast::<ICoreWebView2_15>(&webview).map_err(WindowsError)?;
            let manager = webview.CookieManager().map_err(WindowsError)?;
            GetCookiesCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    manager.GetCookies(w!(""), &handler)?;
                    Ok(())
                }),
                Box::new(move |hresult, list| {
                    hresult?;
                    #[cfg(feature = "tracing")]
                    tracing::info!(?list);
                    done_tx.send(list.map(GetCookiesResult::new)).unwrap();
                    Ok(())
                }),
            )?;
            Ok(())
        }

        let window = self.clone();
        async move {
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            let (call_tx, call_rx) = tokio::sync::oneshot::channel();
            window
                .with_webview(move |webview| unsafe {
                    let result = run(webview, done_tx).map_err(Into::into);
                    call_tx.send(result).unwrap();
                })
                .map_err(Into::<BoxError>::into)
                .and(call_rx.await?)?;
            if let Some(list) = done_rx.await? {
                let list = list.inner().lock().unwrap();
                let mut cookies = Vec::<Cookie>::new();
                unsafe {
                    let count = &mut u32::default();
                    list.Count(count)?;
                    for i in 0 .. *count - 1 {
                        cookies.push(list.GetValueAtIndex(i)?.try_into()?);
                    }
                }
                Ok(Some(cookies))
            } else {
                Ok(None)
            }
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_navigate(&self, url: url::Url) -> Result<(), BoxError> {
        unsafe fn run(webview: PlatformWebview, url: url::Url) -> Result<(), wry::Error> {
            let webview = webview.controller().CoreWebView2().map_err(WindowsError)?;
            let url = PCWSTR::from(&HSTRING::from(url.as_str()));
            webview.Navigate(url).map_err(WindowsError)?;
            Ok(())
        }

        let (call_tx, call_rx) = std::sync::mpsc::channel();
        self.with_webview(move |webview| unsafe {
            let result = run(webview, url).map_err(Into::into);
            call_tx.send(result).unwrap();
        })
        .map_err(Into::into)
        .and(call_rx.recv().unwrap())
    }
}

mod private {
    pub trait Sealed {}
    impl Sealed for tauri::Window {
    }
}
