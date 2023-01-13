#[cfg(feature = "async-graphql")]
use async_graphql::SimpleObject;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::BoxResult;
use regex::Regex;
use url::Url;

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
    pub http_only: bool,
    pub same_site: Option<String>,
    pub secure: bool,
    pub session: bool,
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
        r = r.field("http_only", &self.http_only);
        for same_site in self.same_site.iter() {
            r = r.field("same_site", same_site);
        }
        r = r.field("secure", &self.secure);
        r = r.field("session", &self.session);
        for comment in self.comment.iter() {
            r = r.field("comment", comment);
        }
        for comment_url in self.comment_url.iter() {
            r = r.field("comment_url", comment_url);
        }
        r.finish_non_exhaustive()
    }
}

pub struct CookieUrl {
    pub url: Url,
    pub match_subdomains: bool,
}

impl CookieUrl {
    fn new(url: Url, match_subdomains: bool) -> Self {
        Self { url, match_subdomains }
    }
}

impl From<Url> for CookieUrl {
    fn from(url: Url) -> Self {
        let match_subdomains = false;
        Self { url, match_subdomains }
    }
}

pub struct CookiePattern {
    regex: Regex,
}

impl CookiePattern {
    pub fn is_match(&self, domain: &str) -> bool {
        self.regex.is_match(domain)
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
pub struct CookiePatternBuilder<'a> {
    urls: Option<&'a [CookieUrl]>,
    regex: Option<Regex>,
}

impl<'a> CookiePatternBuilder<'a> {
    pub fn match_urls(mut self, urls: &'a [CookieUrl]) -> CookiePatternBuilder {
        self.regex = None;
        self.urls = urls.into();
        self
    }

    pub fn match_regex(mut self, regex: Regex) -> CookiePatternBuilder<'a> {
        self.urls = None;
        self.regex = regex.into();
        self
    }

    pub fn build(self) -> BoxResult<CookiePattern> {
        use itertools::Itertools;
        if let Some(regex) = self.regex {
            Ok(CookiePattern { regex })
        } else if let Some(urls) = self.urls {
            let pattern = urls
                .into_iter()
                .map(|CookieUrl { url, match_subdomains }| {
                    let scheme = url.scheme();
                    if !["http", "https"].contains(&scheme) {
                        return Err("scheme must be `http` or `https`".into());
                    }
                    let host = url.host().ok_or::<String>(format!("URL {url} has no host"))?;
                    let prefix = if *match_subdomains { r#".*\.?"# } else { "" };
                    let pattern = format!("^{scheme}://{prefix}{host}$");
                    Ok(pattern)
                })
                .intersperse(Ok::<String, String>(String::from("|")))
                .map(|item| item.map_err(Into::into))
                .collect::<BoxResult<String>>()?;
            let regex = Regex::new(&pattern)?;
            Ok(CookiePattern { regex })
        } else {
            let regex = Regex::new(r#"^.*$"#)?;
            Ok(CookiePattern { regex })
        }
    }
}
