use std::ops::Deref;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct SimplePath {
    buf: String,
}

impl SimplePath {
    pub fn split<S: AsRef<str>>(r: &S) -> impl Iterator<Item = &str> {
        r.as_ref().split(&['/', '\\']).filter(|&p| !p.is_empty())
    }

    pub fn new<S: AsRef<str>>(r: S) -> Self {
        let mut rooted = if r.as_ref().starts_with('/') { "/" } else { "" }.to_owned();
        PathJoiner::new(Self::split(&r)).for_each(|p| rooted += p);
        Self { buf: rooted }
    }

    pub fn as_str(self: &Self) -> &str {
        self.as_ref()
    }

    pub fn join<S: AsRef<str>>(self: &Self, r: S) -> Self {
        let other = SimplePath::new(r);
        if other.as_str().starts_with('/') {
            other
        } else {
            let path = String::from_iter(PathJoiner::new(
                [self.as_str(), other.as_str()]
                    .into_iter()
                    .filter(|&p| !p.is_empty()),
            ));
            Self { buf: path }
        }
    }

    pub fn ancestors(self: &Self) -> impl Iterator<Item = &str> {
        PathAncestors::new(self.as_str())
    }
}

impl AsRef<str> for SimplePath {
    fn as_ref(&self) -> &str {
        self.buf.as_ref()
    }
}

impl AsRef<Path> for SimplePath {
    fn as_ref(&self) -> &Path {
        Path::new(self.buf.as_str())
    }
}

impl Deref for SimplePath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        Path::new(self.as_str())
    }
}

impl Into<SimplePath> for &str {
    fn into(self) -> SimplePath {
        SimplePath::new(self)
    }
}

pub struct PathJoiner<'a, I: Iterator<Item = &'a str>> {
    inner: I,
    next: Option<&'a str>,
}

impl<'a, I: Iterator<Item = &'a str>> PathJoiner<'a, I> {
    pub fn new(mut inner: I) -> Self {
        let next = inner.next();
        Self { inner, next }
    }
}

impl<'a, I: Iterator<Item = &'a str>> Iterator for PathJoiner<'a, I> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next {
            Some(part) => {
                self.next = None;
                Some(part)
            }
            None => {
                self.next = self.inner.next();
                match self.next {
                    Some(_) => Some("/"),
                    None => None,
                }
            }
        }
    }
}

struct PathAncestors<'a> {
    inner: &'a str,
    next: Option<usize>,
    done: bool,
}

impl<'a> PathAncestors<'a> {
    fn new(pth: &'a str) -> Self {
        let trimmed_pth = pth.trim_end_matches('/');
        let pth = if trimmed_pth.is_empty() {
            pth
        } else {
            trimmed_pth
        };
        Self {
            inner: pth,
            next: None,
            done: false,
        }
    }
}

impl<'a> Iterator for PathAncestors<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next {
            Some(x) if x > 0 => {
                self.inner = &self.inner[0..x].trim_end_matches('/');
                self.next = self.inner.rfind('/');
            }
            Some(0) if self.inner.len() > 1 => {
                self.inner = &self.inner[0..1];
                self.next = None;
                self.done = true;
            }
            None if !self.done => {
                self.next = self.inner.rfind('/');
            }
            _ => return None,
        }
        if let None = self.next {
            self.done = true;
        }
        Some(self.inner)
    }
}

#[cfg(test)]
mod test {
    use crate::*;

    #[test]
    fn test_path_joiner() {
        let path = "/var/run\\example";
        let iter = SimplePath::split(&path);
        let mut path_joiner = PathJoiner::new(iter);
        assert_eq!(path_joiner.next(), Some("var"));
        assert_eq!(path_joiner.next(), Some("/"));
        assert_eq!(path_joiner.next(), Some("run"));
        assert_eq!(path_joiner.next(), Some("/"));
        assert_eq!(path_joiner.next(), Some("example"));
        assert_eq!(path_joiner.next(), None);
        assert_eq!(path_joiner.next(), None);
    }

    #[test]
    fn test_empty_iter() {
        let iter = "".split("/").take(0);
        let mut path_joiner = PathJoiner::new(iter);
        assert_eq!(path_joiner.next(), None);
        assert_eq!(path_joiner.next(), None);
    }

    #[test]
    fn test_join() {
        let p1 = SimplePath::new("/var/run/");
        let p2 = "test";
        let joined = p1.join(p2);
        assert_eq!(joined.as_str(), "/var/run/test");
    }

    #[test]
    fn test_join_second_rooted() {
        let p1 = SimplePath::new("/var/run/");
        let p2 = "/test";
        let joined = p1.join(p2);
        assert_eq!(joined.as_str(), "/test");
    }

    #[test]
    fn test_join_empty() {
        let p1 = SimplePath::new("/var/run/");
        let p2 = SimplePath::new("");
        let joined1 = p1.join(&p2);
        let joined2 = p2.join(&p1);
        assert_eq!(joined1.as_str(), "/var/run");
        assert_eq!(joined2.as_str(), "/var/run");
    }

    #[test]
    fn test_ancestors() {
        let path = SimplePath::new("/var/run/tmp/dir/");
        let mut iter = path.ancestors();
        assert_eq!(iter.next(), Some("/var/run/tmp/dir"));
        assert_eq!(iter.next(), Some("/var/run/tmp"));
        assert_eq!(iter.next(), Some("/var/run"));
        assert_eq!(iter.next(), Some("/var"));
        assert_eq!(iter.next(), Some("/"));
        assert_eq!(iter.next(), None);

        let path = SimplePath::new("///var/run//tmp/dir////");
        let mut iter = path.ancestors();
        assert_eq!(iter.next(), Some("/var/run/tmp/dir"));
        assert_eq!(iter.next(), Some("/var/run/tmp"));
        assert_eq!(iter.next(), Some("/var/run"));
        assert_eq!(iter.next(), Some("/var"));
        assert_eq!(iter.next(), Some("/"));
        assert_eq!(iter.next(), None);

        let path = SimplePath::new("var/run//tmp/dir////");
        let mut iter = path.ancestors();
        assert_eq!(iter.next(), Some("var/run/tmp/dir"));
        assert_eq!(iter.next(), Some("var/run/tmp"));
        assert_eq!(iter.next(), Some("var/run"));
        assert_eq!(iter.next(), Some("var"));
        assert_eq!(iter.next(), None);

        let path = SimplePath::new("////");
        let mut iter = path.ancestors();
        assert_eq!(iter.next(), Some("/"));
        assert_eq!(iter.next(), None);

        let path = SimplePath::new("");
        let mut iter = path.ancestors();
        assert_eq!(iter.next(), Some(""));
        assert_eq!(iter.next(), None);
    }
}
