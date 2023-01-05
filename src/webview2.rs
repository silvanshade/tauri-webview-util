use crate::{ApiResult, BoxError, BoxResult, Cookie};
use futures::{future::BoxFuture, prelude::*};
use tauri::{window::PlatformWebview, Window};
use url::Url;
use webview2_com::{
    ClearBrowsingDataCompletedHandler,
    Error::WindowsError,
    GetCookiesCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Cookie,
        ICoreWebView2CookieList,
        ICoreWebView2CookieManager,
        ICoreWebView2Profile2,
        ICoreWebView2_13,
        ICoreWebView2_2,
        COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_DOM_STORAGE,
        COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_PROFILE,
        COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_SITE,
        COREWEBVIEW2_BROWSING_DATA_KINDS_BROWSING_HISTORY,
        COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE,
        COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE,
        COREWEBVIEW2_BROWSING_DATA_KINDS_DOWNLOAD_HISTORY,
        COREWEBVIEW2_BROWSING_DATA_KINDS_FILE_SYSTEMS,
        COREWEBVIEW2_BROWSING_DATA_KINDS_GENERAL_AUTOFILL,
        COREWEBVIEW2_BROWSING_DATA_KINDS_INDEXED_DB,
        COREWEBVIEW2_BROWSING_DATA_KINDS_LOCAL_STORAGE,
        COREWEBVIEW2_BROWSING_DATA_KINDS_PASSWORD_AUTOSAVE,
        COREWEBVIEW2_BROWSING_DATA_KINDS_SETTINGS,
        COREWEBVIEW2_BROWSING_DATA_KINDS_WEB_SQL,
        COREWEBVIEW2_COOKIE_SAME_SITE_KIND,
        COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX,
        COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE,
        COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT,
    },
};
use windows::{
    core::{Interface, HSTRING, PWSTR},
    Win32::Foundation::BOOL,
};

impl crate::WebviewExt for Window {
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_clear_cache(&self) -> BoxFuture<BoxResult<()>> {
        unsafe fn run(webview: PlatformWebview, done_tx: oneshot::Sender<()>) -> Result<(), wry::Error> {
            let webview = webview.controller().CoreWebView2().map_err(WindowsError)?;
            let webview = Interface::cast::<ICoreWebView2_13>(&webview).map_err(WindowsError)?;
            let profile = webview.Profile().map_err(WindowsError)?;
            let profile = Interface::cast::<ICoreWebView2Profile2>(&profile).map_err(WindowsError)?;
            ClearBrowsingDataCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    let datakinds = COREWEBVIEW2_BROWSING_DATA_KINDS_FILE_SYSTEMS
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_INDEXED_DB
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_LOCAL_STORAGE
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_WEB_SQL
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_DOM_STORAGE
                        // | COREWEBVIEW2_BROWSING_DATA_KINDS_COOKIES
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_SITE
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_DOWNLOAD_HISTORY
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_GENERAL_AUTOFILL
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_PASSWORD_AUTOSAVE
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_BROWSING_HISTORY
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_SETTINGS
                        | COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_PROFILE;
                    profile.ClearBrowsingData(datakinds, &handler)?;
                    Ok(())
                }),
                Box::new(|hresult| {
                    hresult?;
                    done_tx.send(()).unwrap();
                    Ok(())
                }),
            )?;
            Ok(())
        }

        let window = self.clone();
        async move {
            let (done_tx, done_rx) = oneshot::channel();
            let (call_tx, call_rx) = oneshot::channel();
            window
                .with_webview(move |webview| unsafe {
                    let result = run(webview, done_tx).map_err(Into::into);
                    call_tx.send(result).unwrap();
                })
                .map_err(Into::<BoxError>::into)
                .and(call_rx.await?)?;
            Ok(done_rx.await?)
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_delete_cookies(&self, url: Option<Url>) -> BoxFuture<BoxResult<Vec<Cookie>>> {
        let window = self.clone();
        async move {
            let mut cookies = vec![];
            if let Some(list) = unsafe { webview_get_raw_cookies(&window, url.clone()) }.await? {
                let cookie_manager = unsafe { webview_get_cookie_manager(&window) }.await?;
                let cookie_manager = cookie_manager.lock()?;
                let list = list.lock()?;
                let count = &mut u32::default();
                unsafe {
                    list.Count(count)?;
                    for i in 0 .. *count {
                        let cookie = list.GetValueAtIndex(i)?;
                        cookie_manager.DeleteCookie(&cookie)?;
                        cookies.push(cookie.try_into()?);
                    }
                }
            }
            Ok(cookies)
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_get_cookies(&self, url: Option<Url>) -> BoxFuture<BoxResult<Vec<Cookie>>> {
        let window = self.clone();
        async move {
            if let Some(list) = unsafe { webview_get_raw_cookies(&window, url) }.await? {
                let list = list.lock()?;
                let mut cookies = Vec::<Cookie>::new();
                unsafe {
                    let count = &mut u32::default();
                    list.Count(count)?;
                    for i in 0 .. *count {
                        cookies.push(list.GetValueAtIndex(i)?.try_into()?);
                    }
                }
                Ok(cookies)
            } else {
                Ok(vec![])
            }
        }
        .boxed()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument)]
    fn webview_navigate(&self, url: Url) -> BoxResult<()> {
        unsafe fn run(webview: PlatformWebview, url: Url) -> Result<(), wry::Error> {
            let webview = webview.controller().CoreWebView2().map_err(WindowsError)?;
            let url = &HSTRING::from(url.as_str());
            webview.Navigate(url).map_err(WindowsError)?;
            Ok(())
        }

        let (call_tx, call_rx) = oneshot::channel();
        self.with_webview(move |webview| unsafe {
            let result = run(webview, url).map_err(Into::into);
            call_tx.send(result).unwrap();
        })
        .map_err(Into::into)
        .and(call_rx.recv().unwrap())
    }
}

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

            let name = name.to_string()?.into();
            let value = value.to_string()?.into();
            let domain = domain.to_string()?.into();
            let path = path.to_string()?.into();
            let expires = {
                let expires = expires.round() as i64;
                time::OffsetDateTime::from_unix_timestamp(expires)?
            }
            .into();
            let is_http_only = is_http_only.as_bool().into();
            let same_site = match *same_site {
                COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE => String::from("none"),
                COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX => String::from("lax"),
                COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT => String::from("strict"),
                _ => unreachable!(),
            }
            .into();
            let is_secure = is_secure.as_bool().into();
            let is_session = is_session.as_bool().into();

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

#[cfg_attr(feature = "tracing", tracing::instrument)]
async unsafe fn webview_get_cookie_manager(window: &Window) -> BoxResult<ApiResult<ICoreWebView2CookieManager>> {
    unsafe fn run(webview: PlatformWebview) -> Result<ApiResult<ICoreWebView2CookieManager>, wry::Error> {
        let webview = webview.controller().CoreWebView2().map_err(WindowsError)?;
        let webview = Interface::cast::<ICoreWebView2_2>(&webview).map_err(WindowsError)?;
        let manager = webview.CookieManager().map_err(WindowsError)?;
        Ok(manager.into())
    }

    let (call_tx, call_rx) = oneshot::channel();
    window
        .with_webview(|webview| {
            let result = run(webview).map_err(Into::<BoxError>::into);
            call_tx.send(result).unwrap();
        })
        .map_err(Into::<BoxError>::into)?;
    Ok(call_rx.await??)
}

#[cfg_attr(feature = "tracing", tracing::instrument)]
async unsafe fn webview_get_raw_cookies(
    window: &Window,
    url: Option<Url>,
) -> BoxResult<Option<ApiResult<ICoreWebView2CookieList>>> {
    unsafe fn run(
        webview: PlatformWebview,
        url: Option<Url>,
        done_tx: oneshot::Sender<Option<ApiResult<ICoreWebView2CookieList>>>,
    ) -> Result<(), wry::Error> {
        let webview = webview.controller().CoreWebView2().map_err(WindowsError)?;
        let webview = Interface::cast::<ICoreWebView2_2>(&webview).map_err(WindowsError)?;
        let manager = webview.CookieManager().map_err(WindowsError)?;
        GetCookiesCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| {
                let uri = url.map_or(HSTRING::default(), |url| HSTRING::from(url.as_str()));
                manager.GetCookies(&uri, &handler)?;
                Ok(())
            }),
            Box::new(move |hresult, list| {
                hresult?;
                #[cfg(feature = "tracing")]
                tracing::info!(?list);
                done_tx.send(list.map(Into::into)).unwrap();
                Ok(())
            }),
        )?;
        Ok(())
    }

    let (done_tx, done_rx) = oneshot::channel();
    let (call_tx, call_rx) = oneshot::channel();
    window
        .with_webview(move |webview| unsafe {
            let result = run(webview, url, done_tx).map_err(Into::<BoxError>::into);
            call_tx.send(result).unwrap();
        })
        .map_err(Into::<BoxError>::into)
        .and(call_rx.await?)?;
    Ok(done_rx.await?)
}
