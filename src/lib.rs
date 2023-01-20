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
pub use cookie::{Cookie, CookieHost, CookiePattern, CookiePatternBuilder};

use futures::{future::BoxFuture, stream::BoxStream};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};
use tokio::sync::Mutex;
use url::Url;

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type BoxResult<T> = Result<T, BoxError>;

pub trait WebviewExt: private::WebviewExtSealed {
    fn webview_clear_cache(&self) -> BoxFuture<'static, BoxResult<()>>;
    fn webview_delete_cookies(&self, pattern: Option<CookiePattern>) -> BoxFuture<'static, BoxResult<Vec<Cookie>>>;
    fn webview_get_cookies(&self, pattern: Option<CookiePattern>) -> BoxStream<'static, BoxResult<Cookie>>;
    fn webview_navigate(&self, url: Url) -> BoxResult<()>;
}

mod private {
    pub trait WebviewExtSealed {}
    impl WebviewExtSealed for tauri::Window {
    }
}

#[derive(Clone, Debug)]
struct ApiResult<T>(Arc<Mutex<T>>);

impl<T> ApiResult<T> {
    fn new(value: T) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }
}

impl<T> From<T> for ApiResult<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> Deref for ApiResult<T> {
    type Target = Arc<Mutex<T>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for ApiResult<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

unsafe impl<T> Send for ApiResult<T> {
}
unsafe impl<T> Sync for ApiResult<T> {
}
