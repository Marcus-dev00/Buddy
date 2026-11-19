//! SSRF-safe public web search and readable HTML page extraction helpers.

use super::security::validate_public_url;
use super::truncate_chars;
use std::time::Duration;

fn strip_tag_block(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let open_pat = format!("<{tag}");
    let close_pat = format!("</{tag}>");
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    while i < html.len() {
        if let Some(rel_start) = lower[i..].find(&open_pat) {
            let abs_start = i + rel_start;
            out.push_str(&html[i..abs_start]);
            if let Some(rel_end) = lower[abs_start..].find(&close_pat) {
                i = abs_start + rel_end + close_pat.len();
            } else {
                break;
            }
        } else {
            out.push_str(&html[i..]);
            break;
        }
    }
    out
}

fn decode_basic_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
}

pub fn html_to_readable_text(html: &str, max_chars: usize) -> String {
    let mut cleaned = strip_tag_block(html, "script");
    cleaned = strip_tag_block(&cleaned, "style");
    cleaned = strip_tag_block(&cleaned, "noscript");
    cleaned = strip_tag_block(&cleaned, "svg");

    let mut plain = String::with_capacity(cleaned.len() / 2);
    let mut in_tag = false;
    let mut tag_buf = String::new();
    for ch in cleaned.chars() {
        if in_tag {
            if ch == '>' {
                in_tag = false;
                let tag_lower = tag_buf.trim().to_ascii_lowercase();
                let name = tag_lower
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                if matches!(
                    name,
                    "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article"
                ) {
                    plain.push('\n');
                }
                tag_buf.clear();
            } else if tag_buf.len() < 40 {
                tag_buf.push(ch);
            }
        } else if ch == '<' {
            in_tag = true;
            tag_buf.clear();
        } else {
            plain.push(ch);
        }
    }

    let decoded = decode_basic_entities(&plain);
    let mut lines = Vec::new();
    for raw_line in decoded.lines() {
        let collapsed = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !collapsed.is_empty() {
            lines.push(collapsed);
        }
    }
    truncate_chars(&lines.join("\n"), max_chars)
}

fn url_encode_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len() * 2);
    for b in q.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            b' ' => out.push('+'),
            other => {
                out.push('%');
                out.push_str(&format!("{other:02X}"));
            }
        }
    }
    out
}

fn percent_decode_lossy(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(decode_basic_entities(&tag[start..end]))
}

fn clean_ddg_href(raw_href: &str) -> String {
    let normalized = if raw_href.starts_with("//") {
        format!("https:{raw_href}")
    } else {
        raw_href.to_string()
    };
    if let Some(pos) = normalized.find("uddg=") {
        let rest = &normalized[pos + 5..];
        let end = rest.find('&').unwrap_or(rest.len());
        return percent_decode_lossy(&rest[..end]);
    }
    normalized
}

pub fn parse_ddg_html(html: &str, limit: usize) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    let mut cursor = 0usize;
    while results.len() < limit {
        let Some(pos) = html[cursor..].find("result__a") else {
            break;
        };
        let marker = cursor + pos;
        let a_start = html[..marker].rfind("<a").unwrap_or(marker);
        let Some(tag_end_rel) = html[a_start..].find('>') else {
            break;
        };
        let tag_end = a_start + tag_end_rel;
        let a_tag = &html[a_start..=tag_end];
        let Some(close_rel) = html[tag_end + 1..].find("</a>") else {
            break;
        };
        let title_raw = &html[tag_end + 1..tag_end + 1 + close_rel];
        let title = html_to_readable_text(title_raw, 180);
        let url = extract_attr(a_tag, "href")
            .map(|h| clean_ddg_href(&h))
            .unwrap_or_default();

        let after_link = tag_end + 1 + close_rel + 4;
        let window_end = (after_link + 2000).min(html.len());
        let snippet = if let Some(sp) = html[after_link..window_end].find("result__snippet") {
            let s_pos = after_link + sp;
            if let Some(gt) = html[s_pos..window_end].find('>') {
                let body_start = s_pos + gt + 1;
                let end_tag = html[body_start..window_end]
                    .find("</a>")
                    .or_else(|| html[body_start..window_end].find("</div>"))
                    .unwrap_or((window_end - body_start).min(400));
                html_to_readable_text(&html[body_start..body_start + end_tag], 260)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !title.is_empty() && url.starts_with("http") {
            results.push((title, url, snippet));
        }
        cursor = after_link;
    }
    results
}

pub async fn execute_web_search(query: &str, limit: usize) -> Result<String, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("`query` cannot be empty".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let url = format!("https://html.duckduckgo.com/html/?q={}", url_encode_query(trimmed));
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Web search request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Web search HTTP {status}"));
    }
    let html = resp
        .text()
        .await
        .map_err(|e| format!("Failed reading search response: {e}"))?;
    let hits = parse_ddg_html(&html, limit.clamp(1, 8));
    if hits.is_empty() {
        return Ok(format!("No web search results extracted for `{trimmed}`."));
    }
    let formatted = hits
        .into_iter()
        .enumerate()
        .map(|(idx, (title, link, snippet))| {
            if snippet.is_empty() {
                format!("{}. **{}**\n   {}", idx + 1, title, link)
            } else {
                format!("{}. **{}**\n   {}\n   {}", idx + 1, title, link, snippet)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(formatted)
}

pub async fn execute_fetch_web_page(raw_url: &str, max_chars: usize) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(18))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) BuddyDesktop/1.0")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let mut current = validate_public_url(raw_url, true)?;
    let mut final_resp = None;

    for _hop in 0..=3 {
        let resp = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| format!("Fetch failed: {e}"))?;
        if resp.status().is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "Redirect missing Location header".to_string())?;
            let next_url = current
                .join(loc)
                .map_err(|e| format!("Invalid redirect URL: {e}"))?;
            current = validate_public_url(next_url.as_str(), true)?;
            continue;
        }
        final_resp = Some(resp);
        break;
    }

    let resp = final_resp.ok_or_else(|| "Too many redirects".to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status} from {current}"));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !content_type.is_empty()
        && !content_type.contains("text/")
        && !content_type.contains("json")
        && !content_type.contains("xml")
        && !content_type.contains("javascript")
    {
        return Err(format!("Unsupported binary content type: {content_type}"));
    }

    const MAX_BODY_BYTES: usize = 1_500_000;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed reading response body: {e}"))?;
    let slice = if bytes.len() > MAX_BODY_BYTES {
        &bytes[..MAX_BODY_BYTES]
    } else {
        &bytes[..]
    };
    let raw_text = String::from_utf8_lossy(slice);
    let cap = max_chars.clamp(500, 16_000);
    let body = if content_type.contains("json") || content_type.contains("text/plain") {
        truncate_chars(&raw_text, cap)
    } else {
        html_to_readable_text(&raw_text, cap)
    };
    Ok(format!("URL: {current}\n\n{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_readable_html_and_ddg_results() {
        let html = r#"
          <html><head><style>.a{color:red}</style><script>alert(1)</script></head>
          <body><h1>Rust Docs</h1><p>Hello &amp; welcome!</p></body></html>
        "#;
        let text = html_to_readable_text(html, 500);
        assert!(!text.contains("alert(1)"));
        assert!(text.contains("Rust Docs"));
        assert!(text.contains("Hello & welcome!"));

        let ddg = r#"
          <div class="result">
            <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F&amp;rut=1">Rust Language</a>
            <a class="result__snippet">Systems programming language.</a>
          </div>
        "#;
        let hits = parse_ddg_html(ddg, 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "Rust Language");
        assert_eq!(hits[0].1, "https://rust-lang.org/");
        assert!(hits[0].2.contains("Systems programming"));
    }
}
