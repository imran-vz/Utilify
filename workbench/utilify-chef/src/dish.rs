//! The value flowing between operations, with CyberChef's conversions so
//! chained recipes give the same results.

use std::borrow::Cow;

/// Text, raw bytes or HTML. Each operation asks for the form it works on
/// ([`Dish::bytes`], [`Dish::text`]) and returns whichever it produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dish {
    Bytes(Vec<u8>),
    Text(String),
    /// Markup for display, such as highlighted offsets. As input to the next
    /// operation it is the text with tags stripped and entities decoded.
    Html(String),
}

impl Default for Dish {
    fn default() -> Self {
        Dish::Bytes(Vec::new())
    }
}

impl Dish {
    /// The bytes an operation reading raw data sees. Text whose characters
    /// all fit in a byte becomes those bytes (Latin-1); anything wider is
    /// UTF-8. CyberChef's `strToArrayBuffer`.
    pub fn bytes(&self) -> Cow<'_, [u8]> {
        match self {
            Dish::Bytes(b) => Cow::Borrowed(b),
            Dish::Text(s) => match latin1_bytes(s) {
                Some(b) => Cow::Owned(b),
                None => Cow::Borrowed(s.as_bytes()),
            },
            Dish::Html(h) => Cow::Owned(Dish::Text(html_to_text(h)).bytes().into_owned()),
        }
    }

    /// The text an operation reading a string sees: bytes as UTF-8, or as
    /// Latin-1 when they aren't valid UTF-8. CyberChef's `byteArrayToUtf8`.
    pub fn text(&self) -> Cow<'_, str> {
        match self {
            Dish::Bytes(b) => bytes_to_text(b),
            Dish::Text(s) => Cow::Borrowed(s),
            Dish::Html(h) => Cow::Owned(html_to_text(h)),
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Dish::Bytes(b) => b,
            other => other.bytes().into_owned(),
        }
    }

    pub fn into_text(self) -> String {
        match self {
            Dish::Text(s) => s,
            other => other.text().into_owned(),
        }
    }

    /// What the output pane shows; HTML stays as markup for the caller to
    /// render or strip.
    pub fn display(&self) -> Cow<'_, str> {
        match self {
            Dish::Html(h) => Cow::Borrowed(h),
            other => other.text(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Dish::Bytes(b) => b.len(),
            Dish::Text(s) | Dish::Html(s) => s.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes that aren't valid UTF-8, which the output can only show as
    /// Latin-1 characters.
    pub fn is_binary(&self) -> bool {
        matches!(self, Dish::Bytes(b) if std::str::from_utf8(b).is_err())
    }
}

/// UTF-8 when valid, else one character per byte.
pub fn bytes_to_text(b: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(b) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => Cow::Owned(latin1_to_string(b)),
    }
}

/// One character per byte.
pub fn latin1_to_string(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// `Some` when every character fits in a byte and at least one isn't ASCII
/// (pure ASCII is already its own UTF-8, so no copy is needed).
fn latin1_bytes(s: &str) -> Option<Vec<u8>> {
    if s.is_ascii() {
        return None;
    }
    s.chars().map(|c| u8::try_from(u32::from(c)).ok()).collect()
}

/// CyberChef's `strToArrayBuffer` on a string that's already in hand.
pub fn text_to_bytes(s: &str) -> Vec<u8> {
    latin1_bytes(s).unwrap_or_else(|| s.as_bytes().to_vec())
}

/// HTML as plain text: tags removed (`<br>` as a newline) and the common
/// entities decoded, like CyberChef's DishHTML.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            out.push_str(&rest[lt..]);
            rest = "";
            break;
        };
        let tag = &rest[lt + 1..lt + gt];
        if tag.trim_end_matches('/').trim().eq_ignore_ascii_case("br") {
            out.push('\n');
        }
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(rest);
    unescape_html(&out)
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '/' => out.push_str("&#x2F;"),
            '`' => out.push_str("&#x60;"),
            c => out.push(c),
        }
    }
    out
}

pub fn unescape_html(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
        .replace("&#x60;", "`")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_latin1_until_it_cannot_be() {
        assert_eq!(&*Dish::Text("é".into()).bytes(), &[0xe9]);
        assert_eq!(&*Dish::Text("é你".into()).bytes(), "é你".as_bytes());
        assert_eq!(&*Dish::Text("abc".into()).bytes(), b"abc");
    }

    #[test]
    fn bytes_are_utf8_until_they_cannot_be() {
        assert_eq!(Dish::Bytes("é".as_bytes().to_vec()).text(), "é");
        assert_eq!(Dish::Bytes(vec![0xe9, 0x41]).text(), "éA");
        assert!(Dish::Bytes(vec![0xff]).is_binary());
        assert!(!Dish::Text("ÿ".into()).is_binary());
    }

    #[test]
    fn html_reads_as_text() {
        let h = Dish::Html("<span class='x'>a&amp;b</span><br>c &lt;d&gt;".into());
        assert_eq!(h.text(), "a&b\nc <d>");
        assert_eq!(
            unescape_html(&escape_html("<a href='/x'>&</a>")),
            "<a href='/x'>&</a>"
        );
    }
}
