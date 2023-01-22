// #[cfg(any(
//     target_os = "linux",
//     target_os = "dragonfly",
//     target_os = "freebsd",
//     target_os = "openbsd",
//     target_os = "netbsd"
// ))]
// mod webkit2gtk;

#[cfg(target_os = "macos")]
mod wkwebview;

// #[cfg(target_os = "windows")]
// mod webview2;

mod cookie;
pub use cookie::{Cookie, CookieHost, CookiePattern, CookiePatternBuilder};

use futures::{future::BoxFuture, stream::BoxStream};
use url::Url;

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type BoxResult<T> = Result<T, BoxError>;

pub trait WebViewExt: sealed::WebViewExt {
    fn webview_clear_cache(&self) -> BoxFuture<'static, BoxResult<()>>;
    fn webview_delete_cookies(&self, pattern: CookiePattern) -> BoxFuture<'static, BoxResult<Vec<Cookie>>>;
    fn webview_get_cookies(&self, pattern: CookiePattern) -> BoxResult<BoxStream<'static, BoxResult<Cookie>>>;
    fn webview_navigate(&self, url: Url) -> BoxResult<()>;
}

mod sealed {
    pub trait WebViewExt {}
    impl WebViewExt for tauri::Window {
    }
    pub trait WebViewExtForWKWebView {}
    impl WebViewExtForWKWebView for tauri::Window {
    }
}
