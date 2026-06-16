/// Result of parsing a pathname into components.
///
/// Pathnames are split on `/` boundaries.  An absolute path has a leading
/// slash; a relative path does not.  `.` and `..` components are preserved
/// for the caller to resolve against its current directory context.
#[derive(Clone, Debug)]
pub struct PathComponents<'a> {
    full: &'a str,
    components: [PathComponent; MAX_PATH_DEPTH],
    count: usize,
    absolute: bool,
}

/// Maximum number of components in a single path.
///
/// Deeply nested paths exceeding this limit return `PathError::TooDeep`.
pub const MAX_PATH_DEPTH: usize = 32;

/// Maximum length of a single path component (filename / directory name).
pub const MAX_NAME_LEN: usize = 255;

/// Error returned by pathname parsing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathError {
    /// The path has too many components (> MAX_PATH_DEPTH).
    TooDeep,
    /// A single component exceeds MAX_NAME_LEN.
    NameTooLong,
    /// The path contains an empty component (e.g. `//`).
    EmptyComponent,
}

/// A single component of a path (between `/` separators).
#[derive(Clone, Copy, Debug)]
struct PathComponent {
    offset: u16,
    len: u16,
}

impl PathComponent {
    fn as_str<'a>(&self, full: &'a str) -> &'a str {
        &full[self.offset as usize..(self.offset + self.len) as usize]
    }
}

impl<'a> PathComponents<'a> {
    /// Parse a path string into components.
    ///
    /// Returns `None` if the path is empty ("").
    pub fn parse(path: &'a str) -> Result<Self, PathError> {
        if path.is_empty() {
            return Err(PathError::EmptyComponent);
        }

        let absolute = path.as_bytes()[0] == b'/';
        let mut components = [PathComponent { offset: 0, len: 0 }; MAX_PATH_DEPTH];
        let mut count = 0;
        let bytes = path.as_bytes();

        let mut pos = 0;

        // Skip leading '/'
        if absolute {
            pos = 1;
            // Special case: "/" → one empty-ish root component, but we treat
            // it as 0 components with absolute=true.
            if pos >= bytes.len() {
                return Ok(Self {
                    full: path,
                    components,
                    count: 0,
                    absolute: true,
                });
            }
        }

        while pos < bytes.len() {
            // Find the end of this component
            let start = pos;
            while pos < bytes.len() && bytes[pos] != b'/' {
                pos += 1;
            }

            let len = pos - start;
            if len == 0 {
                // "//" or trailing "/"
                if pos >= bytes.len() {
                    // trailing "/" is OK — just ignore
                    break;
                }
                return Err(PathError::EmptyComponent);
            }
            if len > MAX_NAME_LEN {
                return Err(PathError::NameTooLong);
            }

            if count >= MAX_PATH_DEPTH {
                return Err(PathError::TooDeep);
            }

            components[count] = PathComponent {
                offset: start as u16,
                len: len as u16,
            };
            count += 1;

            // Skip the '/' separator
            pos += 1;
        }

        Ok(Self {
            full: path,
            components,
            count,
            absolute,
        })
    }

    /// Whether the path is absolute (starts with `/`).
    pub fn is_absolute(&self) -> bool {
        self.absolute
    }

    /// Number of components.
    pub fn component_count(&self) -> usize {
        self.count
    }

    /// Get the component at `index`.
    pub fn component(&self, index: usize) -> Option<&str> {
        if index >= self.count {
            return None;
        }
        Some(self.components[index].as_str(self.full))
    }

    /// Iterate over all components in order.
    pub fn components(&self) -> PathComponentIter<'_> {
        PathComponentIter {
            path: self,
            index: 0,
            end: self.count,
        }
    }

    /// The last component (filename), or `None` for root `/`.
    pub fn filename(&self) -> Option<&str> {
        if self.count == 0 {
            None
        } else {
            Some(self.components[self.count - 1].as_str(self.full))
        }
    }

    /// All components except the last one (directory portion).
    pub fn directory_components(&self) -> PathComponentIter<'_> {
        PathComponentIter {
            path: self,
            index: 0,
            end: if self.count > 0 { self.count - 1 } else { 0 },
        }
    }

    /// Whether this path equals `.`.
    pub fn is_dot(&self) -> bool {
        !self.absolute && self.count == 1 && self.components[0].as_str(self.full) == "."
    }

    /// Whether this path equals `..`.
    pub fn is_dotdot(&self) -> bool {
        !self.absolute && self.count == 1 && self.components[0].as_str(self.full) == ".."
    }
}

/// Iterator over path components.
pub struct PathComponentIter<'a> {
    path: &'a PathComponents<'a>,
    index: usize,
    end: usize,
}

impl<'a> Iterator for PathComponentIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.end {
            return None;
        }
        let c = self.path.component(self.index);
        self.index += 1;
        c
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end.saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn absolute_path() {
        let p = PathComponents::parse("/home/user/file.txt").unwrap();
        assert!(p.is_absolute());
        assert_eq!(p.component_count(), 3);
        assert_eq!(p.component(0), Some("home"));
        assert_eq!(p.component(1), Some("user"));
        assert_eq!(p.component(2), Some("file.txt"));
        assert_eq!(p.filename(), Some("file.txt"));
    }

    #[test]
    fn relative_path() {
        let p = PathComponents::parse("foo/bar").unwrap();
        assert!(!p.is_absolute());
        assert_eq!(p.component_count(), 2);
        assert_eq!(p.component(0), Some("foo"));
        assert_eq!(p.component(1), Some("bar"));
    }

    #[test]
    fn root_slash() {
        let p = PathComponents::parse("/").unwrap();
        assert!(p.is_absolute());
        assert_eq!(p.component_count(), 0);
        assert_eq!(p.filename(), None);
    }

    #[test]
    fn single_component() {
        let p = PathComponents::parse("hello").unwrap();
        assert!(!p.is_absolute());
        assert_eq!(p.component_count(), 1);
        assert_eq!(p.component(0), Some("hello"));
    }

    #[test]
    fn dot_and_dotdot() {
        assert!(PathComponents::parse(".").unwrap().is_dot());
        assert!(PathComponents::parse("..").unwrap().is_dotdot());
        assert!(!PathComponents::parse("...").unwrap().is_dot());
    }

    #[test]
    fn trailing_slash() {
        let p = PathComponents::parse("dir/").unwrap();
        assert_eq!(p.component_count(), 1);
        assert_eq!(p.component(0), Some("dir"));
    }

    #[test]
    fn double_slash_rejected() {
        assert!(matches!(
            PathComponents::parse("a//b"),
            Err(PathError::EmptyComponent),
        ));
    }

    #[test]
    fn deep_path() {
        let path = (0..33).map(|_| "a").collect::<Vec<_>>().join("/");
        assert!(matches!(
            PathComponents::parse(&path),
            Err(PathError::TooDeep),
        ));
    }

    #[test]
    fn iteration_yields_all_components() {
        let p = PathComponents::parse("/a/b/c").unwrap();
        let v: Vec<&str> = p.components().collect();
        assert_eq!(v, vec!["a", "b", "c"]);
    }

    #[test]
    fn directory_components_omits_filename() {
        let p = PathComponents::parse("/dir/file.txt").unwrap();
        let dirs: Vec<&str> = p.directory_components().collect();
        assert_eq!(dirs, vec!["dir"]);
    }

    #[test]
    fn empty_path_rejected() {
        assert!(matches!(
            PathComponents::parse(""),
            Err(PathError::EmptyComponent),
        ));
    }

    #[test]
    fn long_component_rejected() {
        let long = "a".repeat(256);
        assert!(matches!(
            PathComponents::parse(&long),
            Err(PathError::NameTooLong),
        ));
    }
}
