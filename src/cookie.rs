#[cfg(target_os = "windows")]
use ::{
    webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Cookie, COREWEBVIEW2_COOKIE_SAME_SITE_KIND},
    windows::{core::PWSTR, Win32::Foundation::BOOL},
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
    type Error = crate::BoxError;

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
