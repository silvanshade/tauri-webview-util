use crate::{BoxError, BoxResult};
#[cfg(feature = "async-graphql")]
use async_graphql::SimpleObject;
#[cfg(feature = "regex")]
use regex::Regex;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};
use tap::prelude::*;
use url::Url;
#[cfg(target_os = "windows")]
use ::{
    webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Cookie,
    windows::core::PWSTR,
    windows::Win32::Foundation::BOOL,
};

#[derive(Debug)]
pub struct CookieError(BoxError);

impl std::fmt::Display for CookieError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for CookieError {
}
unsafe impl Send for CookieError {
}
unsafe impl Sync for CookieError {
}

impl From<BoxError> for CookieError {
    fn from(error: BoxError) -> Self {
        CookieError(error)
    }
}

impl From<String> for CookieError {
    fn from(error: String) -> Self {
        error.conv::<BoxError>().into()
    }
}

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
    #[cfg(feature = "time")]
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
        #[cfg(feature = "time")]
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

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct CookieHost {
    schemes: BTreeSet<CookieHostScheme>,
    host: url::Host,
    matches_subdomains: bool,
}

impl CookieHost {
    pub fn new(host: url::Host) -> Self {
        let schemes = [CookieHostScheme::Http, CookieHostScheme::Https].into_iter().collect();
        let matches_subdomains = false;
        Self {
            schemes,
            host,
            matches_subdomains,
        }
    }

    pub fn with_schemes(mut self, schemes: &[CookieHostScheme]) -> Self {
        self.schemes = schemes.iter().copied().collect();
        self
    }

    pub fn with_subdomains(mut self) -> Self {
        self.matches_subdomains = true;
        self
    }

    pub(crate) fn urls(&self) -> Vec<Url> {
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
        let mut this = CookieHost::from(host);
        this.schemes = [url.scheme().try_into()?].into_iter().collect();
        Ok(this)
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
    pub(crate) hosts: Option<BTreeSet<CookieHost>>,
    pub(crate) matcher: Arc<dyn Fn(&str, bool) -> bool + Send + Sync + 'static>,
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
    #[cfg(feature = "regex")]
    regex: Option<Regex>,
}

impl CookiePatternBuilder {
    pub fn match_hosts(mut self, hosts: Vec<CookieHost>) -> CookiePatternBuilder {
        self.hosts = hosts.into();
        #[cfg(feature = "regex")]
        {
            self.regex = None;
        }
        self
    }

    #[cfg(feature = "regex")]
    pub fn match_regex(mut self, regex: Regex) -> CookiePatternBuilder {
        self.hosts = None;
        self.regex = regex.into();
        self
    }

    #[cfg(feature = "regex")]
    pub fn build(self) -> BoxResult<CookiePattern> {
        if let Some(regex) = self.regex {
            let hosts = None;
            let matcher = Arc::new(move |host: &str, is_secure: bool| {
                if regex.is_match(&format!("https://{host}")) {
                    return true;
                }
                if !is_secure && regex.is_match(&format!("http://{host}")) {
                    return true;
                }
                false
            });
            Ok(CookiePattern { hosts, matcher })
        } else {
            self.build_without_regex()
        }
    }

    #[cfg(not(feature = "regex"))]
    pub fn build(self) -> BoxResult<CookiePattern> {
        self.build_without_regex()
    }

    pub fn build_without_regex(self) -> BoxResult<CookiePattern> {
        match self.hosts {
            None => {
                let hosts = None;
                let matcher = Arc::new(|_: &str, _| true);
                Ok(CookiePattern { hosts, matcher })
            },
            Some(hosts) => {
                let hosts = hosts.into_iter().collect::<BTreeSet<_>>();
                let matcher = Arc::new({
                    let hosts = hosts.clone();
                    move |host: &str, is_secure| {
                        for cookie_host in hosts.iter() {
                            if is_secure && !cookie_host.schemes.contains(&CookieHostScheme::Https) {
                                return false;
                            }
                            if let Some(prefix) = host.strip_suffix(&cookie_host.host.to_string()) {
                                if prefix.is_empty() {
                                    return true;
                                }
                                if prefix.ends_with('.') && cookie_host.matches_subdomains {
                                    return true;
                                }
                            }
                        }
                        false
                    }
                });
                Ok(CookiePattern {
                    hosts: hosts.into(),
                    matcher,
                })
            },
        }
    }
}
