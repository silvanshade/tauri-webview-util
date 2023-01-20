use std::collections::BTreeSet;

#[cfg(feature = "async-graphql")]
use async_graphql::SimpleObject;

use icrate::Foundation::NSHTTPCookie;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{BoxError, BoxResult};
use regex::Regex;
use tap::prelude::*;
use url::Url;

#[cfg(target_os = "windows")]
use ::{
    webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Cookie,
    windows::core::PWSTR,
    windows::Win32::Foundation::BOOL,
};

#[cfg_attr(feature = "async-graphql", derive(SimpleObject))]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub port_list: Option<Vec<u16>>,
    pub expires: Option<time::OffsetDateTime>,
    pub is_http_only: bool,
    pub same_site: Option<String>,
    pub is_secure: bool,
    pub is_session: Option<bool>,
    pub comment: Option<String>,
    pub comment_url: Option<Url>,
}

impl std::fmt::Display for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        struct Value<'a>(&'a str);
        impl<'a> std::fmt::Debug for Value<'a> {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("<...>")
            }
        }

        let mut r = &mut f.debug_struct("Cookie");
        r = r.field("name", &self.name);
        r = r.field("value", &Value(&self.value));
        r = r.field("domain", &self.domain);
        r = r.field("path", &self.path);
        for port_list in self.port_list.iter() {
            if !port_list.is_empty() {
                r = r.field("port_list", port_list);
            }
        }
        for expires in self.expires.iter() {
            r = r.field("expires", expires);
        }
        r = r.field("is_http_only", &self.is_http_only);
        for same_site in self.same_site.iter() {
            r = r.field("same_site", same_site);
        }
        r = r.field("is_secure", &self.is_secure);
        r = r.field("is_session", &self.is_session);
        for comment in self.comment.iter() {
            r = r.field("comment", comment);
        }
        for comment_url in self.comment_url.iter() {
            r = r.field("comment_url", comment_url);
        }
        r.finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum CookieHostScheme {
    Http,
    Https,
}

impl std::fmt::Display for CookieHostScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CookieHostScheme::Http => write!(f, "http"),
            CookieHostScheme::Https => write!(f, "https"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CookieHost {
    host: url::Host,
    schemes: BTreeSet<CookieHostScheme>,
}

impl CookieHost {
    pub fn new(host: url::Host) -> Self {
        let schemes = [CookieHostScheme::Http, CookieHostScheme::Https].into_iter().collect();
        Self { host, schemes }
    }

    pub fn with_schemes(mut self, schemes: &[CookieHostScheme]) -> Self {
        self.schemes = schemes.iter().copied().collect();
        self
    }

    pub fn urls(&self) -> Vec<Url> {
        let mut urls = vec![];
        for scheme in self.schemes.iter() {
            let host = &self.host;
            let url = Url::parse(&format!("{scheme}://{host}")).expect("parsing should never fail");
            urls.push(url);
        }
        urls
    }
}

impl From<url::Host> for CookieHost {
    fn from(host: url::Host) -> Self {
        Self::new(host)
    }
}

impl TryFrom<Url> for CookieHost {
    type Error = BoxError;

    fn try_from(url: Url) -> Result<Self, Self::Error> {
        let host = url.host().ok_or(format!(r#"url "{url}" has no host"#))?.to_owned();
        let scheme = url.scheme();
        CookieHost::from(host).pipe(Ok)
    }
}

impl TryFrom<&str> for CookieHostScheme {
    type Error = BoxError;

    fn try_from(scheme: &str) -> Result<Self, Self::Error> {
        match scheme {
            "http" => Ok(CookieHostScheme::Http),
            "https" => Ok(CookieHostScheme::Https),
            _ => Err(format!(r#"scheme `{scheme}` not supported"#).into()),
        }
    }
}

#[derive(Clone)]
pub struct CookiePattern {
    hosts: Option<Vec<CookieHost>>,
    regex: Regex,
}

impl CookiePattern {
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    pub(crate) fn cookie_matches(&self, cookie: &mut soup::Cookie) -> BoxResult<bool> {
        fn unexpectedly_null(field: &str) -> BoxError {
            format!("field `{field}` unexpectedly null").into()
        }
        let domain = cookie
            .domain()
            .map(Into::<String>::into)
            .ok_or(unexpectedly_null("domain"))?;
        let domain = domain.trim_start_matches('.');
        let is_secure = cookie.is_secure();
        let scheme = if is_secure { "https" } else { "http" };
        let url = Url::parse(&format!("{scheme}://{domain}"))?;
        Ok(self.regex.is_match(url.as_str()))
    }

    pub(crate) fn cookie_matches(&self, cookie: &NSHTTPCookie) -> BoxResult<bool> {
        let domain = unsafe { cookie.domain() }.to_string();
        let domain = domain.trim_start_matches('.');
        let secure = unsafe { cookie.isSecure() };
        let scheme = if secure { "https" } else { "http" };
        let url = format!("{scheme}://{domain}");
        Ok(self.regex.is_match(url.as_str()))
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn cookie_matches(&self, cookie: &ICoreWebView2Cookie) -> BoxResult<bool> {
        let domain = unsafe {
            let ptr = &mut PWSTR::null();
            cookie.Domain(ptr)?;
            ptr.to_string()?
        };
        let domain = domain.trim_start_matches('.');
        let is_secure = unsafe {
            let ptr = &mut BOOL::default();
            cookie.IsSecure(ptr)?;
            ptr.as_bool()
        };
        let scheme = if is_secure { "https" } else { "http" };
        let url = Url::parse(&format!("{scheme}://{domain}"))?;
        Ok(self.regex.is_match(url.as_str()))
    }
}

impl Default for CookiePattern {
    fn default() -> Self {
        if let Ok(pattern) = CookiePatternBuilder::default().build() {
            return pattern;
        } else {
            unreachable!()
        }
    }
}

#[derive(Default)]
pub struct CookiePatternBuilder {
    hosts: Option<Vec<CookieHost>>,
    regex: Option<Regex>,
    match_subdomains: bool,
}

impl CookiePatternBuilder {
    pub fn match_hosts(mut self, hosts: Vec<CookieHost>) -> CookiePatternBuilder {
        self.hosts = hosts.into();
        self.regex = None;
        self
    }

    pub fn match_regex(mut self, regex: Regex) -> CookiePatternBuilder {
        self.hosts = None;
        self.regex = regex.into();
        self
    }

    pub fn build(self) -> BoxResult<CookiePattern> {
        #![allow(unstable_name_collisions)]
        use itertools::Itertools;
        let prefix = if self.match_subdomains { r#".*\.?"# } else { "" };
        if let Some(regex) = self.regex {
            let hosts = None;
            Ok(CookiePattern { hosts, regex })
        } else if let Some(hosts) = &self.hosts {
            let pattern = hosts
                .iter()
                .flat_map(|cookie_host| {
                    let CookieHost { host, schemes } = cookie_host;
                    std::iter::empty()
                        .chain(Some(String::from("(?:")))
                        .chain({
                            schemes
                                .into_iter()
                                .map(move |scheme| format!("^{scheme}://{prefix}{host}/$"))
                                .intersperse(String::from("|"))
                        })
                        .chain(Some(String::from(")")))
                })
                .intersperse(String::from("|"))
                .collect::<String>();
            let hosts = if self.match_subdomains { None } else { self.hosts };
            let regex = Regex::new(&pattern)?;
            Ok(CookiePattern { hosts, regex })
        } else {
            let hosts = None;
            let regex = Regex::new(r#"^.*$"#)?;
            Ok(CookiePattern { hosts, regex })
        }
    }
}
