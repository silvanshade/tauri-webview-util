#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
mod webkit2gtk;

#[cfg(target_os = "macos")]
mod wkwebview;

#[cfg(target_os = "windows")]
mod webview2;

mod cookie;
pub use cookie::{Cookie, CookiePattern, CookiePatternBuilder, CookieUrl};

use futures::future::BoxFuture;
use std::sync::{Arc, Mutex, MutexGuard};
use url::Url;

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type BoxResult<T> = Result<T, BoxError>;

pub trait WebviewExt: private::WebviewExtSealed {
    fn webview_clear_cache(&self) -> BoxFuture<BoxResult<()>>;
    fn webview_delete_cookies(&self, pattern: Option<CookiePattern>) -> BoxFuture<BoxResult<Vec<Cookie>>>;
    fn webview_get_cookies(&self, pattern: Option<CookiePattern>) -> BoxFuture<BoxResult<Vec<Cookie>>>;
    fn webview_navigate(&self, url: Url) -> BoxResult<()>;
}

mod private {
    pub trait WebviewExtSealed {}
    impl WebviewExtSealed for tauri::Window {
    }
}

#[derive(Debug)]
struct ApiResult<T>(Arc<Mutex<T>>);

impl<T> Clone for ApiResult<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> ApiResult<T> {
    fn new(value: T) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }

    fn lock(&self) -> BoxResult<MutexGuard<T>> {
        self.0.lock().map_err(|err| err.to_string().into())
    }
}

impl<T> From<T> for ApiResult<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

unsafe impl<T> Send for ApiResult<T> {
}
unsafe impl<T> Sync for ApiResult<T> {
}
