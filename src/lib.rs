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

use futures::{future::BoxFuture, stream::BoxStream};
use std::ops::{Deref, DerefMut};
use url::Url;

pub type BoxError<'a> = Box<dyn std::error::Error + Send + Sync + 'a>;
pub type BoxResult<'a, T> = Result<T, BoxError<'a>>;

pub trait WebviewExt: private::WebviewExtSealed {
    fn webview_clear_cache(&self) -> BoxFuture<BoxResult<()>>;
    fn webview_delete_cookies(&self, pattern: Option<CookiePattern>) -> BoxFuture<BoxResult<Vec<Cookie>>>;
    fn webview_get_cookies(&self, pattern: Option<CookiePattern>) -> BoxStream<BoxResult<Cookie>>;
    fn webview_navigate(&self, url: Url) -> BoxResult<()>;
}

mod private {
    pub trait WebviewExtSealed {}
    impl WebviewExtSealed for tauri::Window {
    }
}

#[derive(Debug)]
struct ApiResult<T>(T);

impl<T> ApiResult<T> {
    fn new(value: T) -> Self {
        Self(value)
    }
}

impl<T> From<T> for ApiResult<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> Deref for ApiResult<T> {
    type Target = T;

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
