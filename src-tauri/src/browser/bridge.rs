//! Windows WebView2 COM bridge (`ICoreWebView2::ExecuteScript` & `CapturePreview`)
//! plus isolated DOM extraction / interaction scripts (zero Tauri IPC exposed to pages).

use crate::ai_client::encode_dynamic_image_jpeg_base64;
use serde_json::Value;
use std::time::Duration;
use tauri::WebviewWindow;
use tokio::sync::oneshot;

#[cfg(windows)]
use webview2_com::{
    CapturePreviewCompletedHandler, ExecuteScriptCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_8, COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    },
};
#[cfg(windows)]
use windows_core_wv::{Interface, PCWSTR};
#[cfg(windows)]
use windows_wv::Win32::Foundation::HGLOBAL;
#[cfg(windows)]
use windows_wv::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
#[cfg(windows)]
use windows_wv::Win32::System::Com::{IStream, STATSTG, STATFLAG_NONAME, STREAM_SEEK_SET};

#[cfg(windows)]
fn decode_exec_result_json(raw: &str) -> Value {
    if raw.is_empty() || raw == "null" {
        return Value::Null;
    }
    serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

#[cfg(windows)]
pub async fn eval_json(window: &WebviewWindow, script: &str) -> Result<Value, String> {
    let (tx, rx) = oneshot::channel::<Result<Value, String>>();
    let mut tx_opt = Some(tx);
    let script_u16: Vec<u16> = script.encode_utf16().chain(std::iter::once(0)).collect();

    window
        .with_webview(move |platform| {
            let result: Result<(), String> = unsafe {
                let ctrl = platform.controller();
                let core = match ctrl.CoreWebView2() {
                    Ok(c) => c,
                    Err(e) => {
                        if let Some(s) = tx_opt.take() {
                            let _ = s.send(Err(format!("CoreWebView2 unavailable: {e}")));
                        }
                        return;
                    }
                };
                let handler = ExecuteScriptCompletedHandler::create(Box::new(move |err, res| {
                    if let Some(s) = tx_opt.take() {
                        let out = if let Err(e) = err {
                            Err(format!("ExecuteScript error: {e}"))
                        } else {
                            Ok(decode_exec_result_json(&res))
                        };
                        let _ = s.send(out);
                    }
                    Ok(())
                }));
                core.ExecuteScript(PCWSTR(script_u16.as_ptr()), &handler)
                    .map_err(|e| format!("ExecuteScript invoke failed: {e}"))
            };
            if let Err(e) = result {
                crate::logger::log_warn("browser", &e);
            }
        })
        .map_err(|e| format!("with_webview failed: {e}"))?;

    match tokio::time::timeout(Duration::from_secs(12), rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => Err("WebView script channel dropped".to_string()),
        Err(_) => Err("Browser script execution timed out".to_string()),
    }
}

#[cfg(not(windows))]
pub async fn eval_json(_window: &WebviewWindow, _script: &str) -> Result<Value, String> {
    Err("Built-in WebView2 bridge is only supported on Windows".to_string())
}

#[cfg(windows)]
unsafe fn read_all_istream(stream: &IStream) -> Result<Vec<u8>, String> {
    let mut stat = STATSTG::default();
    stream
        .Stat(&mut stat, STATFLAG_NONAME)
        .map_err(|e| format!("IStream::Stat failed: {e}"))?;
    let total = stat.cbSize as usize;
    stream
        .Seek(0, STREAM_SEEK_SET, None)
        .map_err(|e| format!("IStream::Seek failed: {e}"))?;
    let mut buf = vec![0u8; total];
    let mut offset = 0usize;
    while offset < total {
        let mut read_len = 0u32;
        let hr = stream.Read(
            buf[offset..].as_mut_ptr() as *mut _,
            (total - offset) as u32,
            Some(&mut read_len),
        );
        if hr.is_err() || read_len == 0 {
            break;
        }
        offset += read_len as usize;
    }
    buf.truncate(offset);
    Ok(buf)
}

#[cfg(windows)]
pub async fn capture_preview_jpeg(
    window: &WebviewWindow,
    max_dim: u32,
    quality: u8,
) -> Result<String, String> {
    let (tx, rx) = oneshot::channel::<Result<Vec<u8>, String>>();
    let tx_shared = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));

    window
        .with_webview(move |platform| unsafe {
            let send_res = |val: Result<Vec<u8>, String>| {
                if let Ok(mut g) = tx_shared.lock() {
                    if let Some(s) = g.take() {
                        let _ = s.send(val);
                    }
                }
            };
            let ctrl = platform.controller();
            let core = match ctrl.CoreWebView2() {
                Ok(c) => c,
                Err(e) => {
                    send_res(Err(format!("CoreWebView2 unavailable: {e}")));
                    return;
                }
            };
            let stream = match CreateStreamOnHGlobal(HGLOBAL::default(), true) {
                Ok(st) => st,
                Err(e) => {
                    send_res(Err(format!("CreateStreamOnHGlobal failed: {e}")));
                    return;
                }
            };
            let stream_read = stream.clone();
            let tx_cb = tx_shared.clone();
            let handler = CapturePreviewCompletedHandler::create(Box::new(move |err| {
                let res = if let Err(e) = err {
                    Err(format!("CapturePreview error: {e}"))
                } else {
                    read_all_istream(&stream_read)
                };
                if let Ok(mut g) = tx_cb.lock() {
                    if let Some(s) = g.take() {
                        let _ = s.send(res);
                    }
                }
                Ok(())
            }));
            if let Err(e) = core.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            ) {
                send_res(Err(format!("CapturePreview invoke failed: {e}")));
            }
        })
        .map_err(|e| format!("with_webview failed: {e}"))?;

    let png_bytes = match tokio::time::timeout(Duration::from_secs(6), rx).await {
        Ok(Ok(Ok(b))) => b,
        Ok(Ok(Err(e))) => return Err(e),
        Ok(Err(_)) => return Err("Preview capture channel closed".to_string()),
        Err(_) => return Err("Preview capture timed out".to_string()),
    };

    if png_bytes.is_empty() {
        return Err("Empty browser preview frame".to_string());
    }

    tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&png_bytes)
            .map_err(|e| format!("Decode preview PNG failed: {e}"))?;
        encode_dynamic_image_jpeg_base64(&img, max_dim, quality)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(not(windows))]
pub async fn capture_preview_jpeg(
    _window: &WebviewWindow,
    _max_dim: u32,
    _quality: u8,
) -> Result<String, String> {
    Err("Built-in WebView2 capture is only supported on Windows".to_string())
}

#[cfg(windows)]
pub fn set_muted(window: &WebviewWindow, muted: bool) {
    let _ = window.with_webview(move |platform| unsafe {
        let ctrl = platform.controller();
        if let Ok(core) = ctrl.CoreWebView2() {
            if let Ok(core8) = core.cast::<ICoreWebView2_8>() {
                let _ = core8.SetIsMuted(muted);
            }
        }
    });
}

#[cfg(not(windows))]
pub fn set_muted(_window: &WebviewWindow, _muted: bool) {}

pub fn build_read_page_script(max_text_chars: usize) -> String {
    format!(
        r#"(() => {{
  const title = (document.title || '').trim();
  const url = location.href;
  const readyState = document.readyState;
  const canonEl = document.querySelector('link[rel="canonical"], meta[property="og:url"]');
  const canonicalUrl = canonEl ? (canonEl.getAttribute('href') || canonEl.getAttribute('content') || '').trim() : '';

  const clone = document.body ? document.body.cloneNode(true) : null;
  let text = '';
  if (clone) {{
    clone.querySelectorAll('script,style,noscript,svg,template').forEach(n => n.remove());
    text = (clone.innerText || clone.textContent || '')
      .replace(/[ \t]+/g, ' ')
      .replace(/\n{{3,}}/g, '\n\n')
      .trim();
  }}
  if (text.length > {max_text_chars}) {{
    text = text.slice(0, {max_text_chars}) + '\n...[truncated]';
  }}

  const sel = 'input,textarea,select,a[href],button,[role="button"],[role="link"]';
  const rawNodes = Array.from(document.querySelectorAll(sel));
  const scored = [];
  for (const el of rawNodes) {{
    const rect = el.getBoundingClientRect();
    if (rect.width < 6 || rect.height < 6) continue;
    const st = window.getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden' || parseFloat(st.opacity || '1') <= 0.05) continue;
    const tag = el.tagName.toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (tag === 'input' && type === 'hidden') continue;
    const label = (el.innerText || el.getAttribute('aria-label') || el.getAttribute('title') || el.getAttribute('placeholder') || el.getAttribute('value') || '').replace(/\s+/g, ' ').trim().slice(0, 100);
    const href = tag === 'a' ? (el.href || el.getAttribute('href') || '').slice(0, 260) : '';
    if (!label && !href && tag !== 'input' && tag !== 'textarea') continue;

    // Prioritize search inputs & main-content cards above header/footer navigation links
    let prio = 2;
    if (tag === 'input' || tag === 'textarea') {{
      prio = (type === 'search' || /search|搜|query|keyword/i.test(label + ' ' + (el.name || '') + ' ' + (el.id || ''))) ? 0 : 1;
    }} else if (el.closest('main, article, [role="main"], #contents, #primary, .search-body, .video-list, .bili-video-card')) {{
      prio = 1;
    }} else if (el.closest('footer, nav')) {{
      prio = 3;
    }}
    scored.push({{ el, tag, type, label, href, prio, top: rect.top }});
  }}
  scored.sort((a, b) => (a.prio - b.prio) || (a.top - b.top));

  const elements = [];
  let idx = 1;
  for (const item of scored) {{
    if (elements.length >= 75) break;
    const bid = 'e' + idx;
    idx++;
    item.el.setAttribute('data-buddy-id', bid);
    elements.push({{
      id: bid,
      tag: item.tag,
      type: item.type,
      label: item.label,
      href: item.href,
      is_password: item.tag === 'input' && item.type === 'password'
    }});
  }}

  const titleLow = title.toLowerCase();
  const pathLow = (location.pathname || '').toLowerCase();
  const isInterstitialChallenge = !!(
    document.querySelector('#cf-challenge-running, .cf-browser-verification, #challenge-running') ||
    (text.length < 1200 && (
      titleLow.includes('just a moment') ||
      titleLow.includes('attention required') ||
      titleLow.includes('安全验证') ||
      titleLow.includes('人机验证') ||
      titleLow.includes('verify you are human')
    ))
  );
  const hasVisibleBlockingCaptcha = Array.from(
    document.querySelectorAll('iframe[src*="recaptcha/api2/bframe"], iframe[src*="hcaptcha.com/captcha"], iframe[src*="challenges.cloudflare.com"]')
  ).some(el => {{
    const r = el.getBoundingClientRect();
    const st = window.getComputedStyle(el);
    return r.width >= 180 && r.height >= 120 && st.display !== 'none' && st.visibility !== 'hidden' && parseFloat(st.opacity || '1') > 0.1;
  }});
  const hasVisiblePassword = Array.from(document.querySelectorAll('input[type="password"]')).some(el => {{
    const r = el.getBoundingClientRect();
    const st = window.getComputedStyle(el);
    return r.width >= 40 && r.height >= 16 && st.display !== 'none' && st.visibility !== 'hidden';
  }});
  const isLoginWall = !isInterstitialChallenge && !hasVisibleBlockingCaptcha && hasVisiblePassword && (
    /\/(login|signin|sign-in|passport|auth)\b/.test(pathLow) || text.length < 450
  );

  const buddyLogs = Array.isArray(window.__buddyConsoleLogs) ? window.__buddyConsoleLogs : [];
  let consoleErrors = 0;
  let consoleWarns = 0;
  for (const entry of buddyLogs) {{
    if (entry && (entry.level === 'error' || entry.level === 'network')) consoleErrors++;
    else if (entry && entry.level === 'warn') consoleWarns++;
  }}

  return {{
    title,
    url,
    canonical_url: canonicalUrl,
    ready_state: readyState,
    scroll_y: Math.round(window.scrollY),
    scroll_height: Math.round(document.documentElement ? document.documentElement.scrollHeight : 0),
    viewport_height: Math.round(window.innerHeight),
    text,
    elements,
    console_error_count: consoleErrors,
    console_warn_count: consoleWarns,
    needs_attention: isInterstitialChallenge || hasVisibleBlockingCaptcha || isLoginWall,
    attention_reason: (isInterstitialChallenge || hasVisibleBlockingCaptcha) ? 'captcha_or_challenge' : (isLoginWall ? 'login_required' : '')
  }};
}})()"#
    )
}

pub fn build_click_script(target: &str) -> Result<String, String> {
    let target_json = serde_json::to_string(target.trim()).map_err(|e| e.to_string())?;
    Ok(format!(
        r#"(() => {{
  const t = {target_json};
  let el = null;
  if (/^e\d+$/i.test(t)) {{
    el = document.querySelector('[data-buddy-id="' + t.toLowerCase() + '"]');
  }}
  if (!el) {{
    try {{ el = document.querySelector(t); }} catch {{}}
  }}
  if (!el) {{
    const low = t.toLowerCase();
    const candidates = document.querySelectorAll('a,button,[role="button"],[role="link"],input[type="submit"],input[type="button"]');
    for (const c of candidates) {{
      const txt = (c.innerText || c.getAttribute('aria-label') || c.getAttribute('title') || '').trim().toLowerCase();
      if (txt && txt.includes(low)) {{ el = c; break; }}
    }}
  }}
  if (!el) return {{ ok: false, error: 'Element not found: ' + t }};
  if (el.tagName && el.tagName.toLowerCase() === 'input' && (el.type || '').toLowerCase() === 'password') {{
    return {{ ok: false, sensitive_password: true, error: 'Password input requires manual user action.' }};
  }}
  el.scrollIntoView({{ block: 'center', inline: 'nearest', behavior: 'instant' }});
  const rect = el.getBoundingClientRect();
  const ring = document.createElement('div');
  ring.style.cssText = 'position:fixed;z-index:2147483647;pointer-events:none;border:2px solid #38bdf8;border-radius:8px;box-shadow:0 0 0 4px rgba(56,189,248,0.28);transition:opacity 0.45s ease;';
  ring.style.left = Math.max(0, rect.left - 3) + 'px';
  ring.style.top = Math.max(0, rect.top - 3) + 'px';
  ring.style.width = Math.max(12, rect.width + 6) + 'px';
  ring.style.height = Math.max(12, rect.height + 6) + 'px';
  document.documentElement.appendChild(ring);
  setTimeout(() => {{ ring.style.opacity = '0'; setTimeout(() => ring.remove(), 480); }}, 450);

  const label = (el.innerText || el.getAttribute('aria-label') || el.getAttribute('title') || el.tagName).replace(/\s+/g, ' ').trim().slice(0, 80);
  const href = el.href || '';
  if (el.getAttribute('target') === '_blank') {{
    el.setAttribute('target', '_self');
  }}
  el.focus({{ preventScroll: true }});
  el.click();
  return {{ ok: true, clicked_label: label, href, url: location.href }};
}})()"#
    ))
}

pub fn build_type_script(target: &str, text: &str, submit: bool) -> Result<String, String> {
    let target_json = serde_json::to_string(target.trim()).map_err(|e| e.to_string())?;
    let text_json = serde_json::to_string(text).map_err(|e| e.to_string())?;
    let submit_lit = if submit { "true" } else { "false" };
    Ok(format!(
        r#"(() => {{
  const t = {target_json};
  const val = {text_json};
  const doSubmit = {submit_lit};
  let el = null;
  if (/^e\d+$/i.test(t)) {{
    el = document.querySelector('[data-buddy-id="' + t.toLowerCase() + '"]');
  }}
  if (!el) {{
    try {{ el = document.querySelector(t); }} catch {{}}
  }}
  if (!el) {{
    const low = t.toLowerCase();
    for (const c of document.querySelectorAll('input,textarea,[contenteditable="true"]')) {{
      const hint = (c.getAttribute('placeholder') || c.getAttribute('aria-label') || c.getAttribute('name') || '').toLowerCase();
      if (hint.includes(low)) {{ el = c; break; }}
    }}
  }}
  if (!el) return {{ ok: false, error: 'Input element not found: ' + t }};
  if (el.tagName && el.tagName.toLowerCase() === 'input' && (el.type || '').toLowerCase() === 'password') {{
    return {{ ok: false, sensitive_password: true, error: 'Refused automated typing into password field. Please type manually in the browser window.' }};
  }}
  el.scrollIntoView({{ block: 'center', inline: 'nearest', behavior: 'instant' }});
  el.focus({{ preventScroll: true }});

  if (el.isContentEditable) {{
    el.textContent = val;
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
  }} else {{
    const proto = el.tagName.toLowerCase() === 'textarea' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const desc = Object.getOwnPropertyDescriptor(proto, 'value');
    if (desc && desc.set) desc.set.call(el, val);
    else el.value = val;
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  }}

  if (doSubmit) {{
    el.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true }}));
    el.dispatchEvent(new KeyboardEvent('keyup', {{ key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true }}));
    if (el.form && typeof el.form.requestSubmit === 'function') {{
      try {{ el.form.requestSubmit(); }} catch {{}}
    }}
  }}
  return {{ ok: true, url: location.href }};
}})()"#
    ))
}

pub fn build_scroll_script(direction: &str, amount: i32) -> String {
    match direction.trim().to_ascii_lowercase().as_str() {
        "top" => "(() => { window.scrollTo({ top: 0, behavior: 'smooth' }); return { scroll_y: 0 }; })()".to_string(),
        "bottom" => "(() => { const h = document.documentElement.scrollHeight; window.scrollTo({ top: h, behavior: 'smooth' }); return { scroll_y: h }; })()".to_string(),
        "up" => format!("(() => {{ window.scrollBy({{ top: -{}, behavior: 'smooth' }}); return {{ scroll_y: Math.round(window.scrollY) }}; }})()", amount.clamp(120, 3000)),
        _ => format!("(() => {{ window.scrollBy({{ top: {}, behavior: 'smooth' }}); return {{ scroll_y: Math.round(window.scrollY) }}; }})()", amount.clamp(120, 3000)),
    }
}

/// Runs before any page script via WebView2 `initialization_script` to passively capture
/// JS runtime exceptions, `console.error`/`warn`, unhandled rejections, and failed `fetch` calls.
pub const CONSOLE_CAPTURE_INIT_JS: &str = r#"(() => {
  if (window.__buddyConsoleHooked) return;
  window.__buddyConsoleHooked = true;
  const MAX = 80;
  const logs = window.__buddyConsoleLogs || (window.__buddyConsoleLogs = []);
  const fmt = (a) => {
    if (a instanceof Error) return (a.stack || a.message || String(a)).slice(0, 600);
    if (typeof a === 'string') return a.slice(0, 500);
    try { return JSON.stringify(a).slice(0, 400); } catch { return String(a).slice(0, 400); }
  };
  const push = (level, msg) => {
    try {
      logs.push({ level, msg: String(msg || '').slice(0, 800), ts: Date.now() });
      if (logs.length > MAX) logs.splice(0, logs.length - MAX);
    } catch {}
  };
  for (const lvl of ['error', 'warn']) {
    const orig = console[lvl];
    if (typeof orig === 'function') {
      console[lvl] = function(...args) {
        push(lvl, args.map(fmt).join(' '));
        return orig.apply(this, args);
      };
    }
  }
  window.addEventListener('error', (ev) => {
    const loc = ev.filename ? ` (${ev.filename}:${ev.lineno}:${ev.colno})` : '';
    const detail = ev.error && ev.error.stack ? ev.error.stack : (ev.message || 'Runtime error');
    push('error', `${detail}${loc}`);
  });
  window.addEventListener('unhandledrejection', (ev) => {
    push('error', 'UnhandledPromiseRejection: ' + fmt(ev.reason));
  });
  if (typeof window.fetch === 'function') {
    const origFetch = window.fetch.bind(window);
    window.fetch = function(...args) {
      const reqUrl = typeof args[0] === 'string' ? args[0] : (args[0] && args[0].url ? args[0].url : '');
      return origFetch(...args).then((res) => {
        if (res && !res.ok && res.status >= 400) {
          push('network', `HTTP ${res.status} ${res.statusText || ''} — ${reqUrl}`);
        }
        return res;
      }).catch((err) => {
        push('network', `Fetch failed (${reqUrl}): ${fmt(err)}`);
        throw err;
      });
    };
  }
})();"#;

pub fn build_read_console_script(clear: bool) -> String {
    let clear_lit = if clear { "true" } else { "false" };
    format!(
        r#"(() => {{
  const arr = Array.isArray(window.__buddyConsoleLogs) ? window.__buddyConsoleLogs.slice() : [];
  if ({clear_lit} && Array.isArray(window.__buddyConsoleLogs)) {{
    window.__buddyConsoleLogs.length = 0;
  }}
  return {{
    url: location.href,
    title: document.title || '',
    count: arr.length,
    logs: arr
  }};
}})()"#
    )
}
