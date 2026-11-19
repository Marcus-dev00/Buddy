import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  ArrowRight,
  Download,
  Eye,
  EyeOff,
  Globe,
  Hand,
  Music,
  Play,
  RotateCw,
  Sparkles,
  Star,
  X,
} from "lucide-react";
import { getStrings } from "../i18n";
import type { BrowserPreviewPayload, BrowserStatePayload, UiLang } from "../types";

const BROWSER_MEDIA_URL_RE =
  /^https?:\/\/(?:www\.|m\.|music\.)?(?:youtube\.com|youtu\.be|bilibili\.com|b23\.tv|soundcloud\.com|music\.163\.com|y\.qq\.com|vimeo\.com|dailymotion\.com|twitch\.tv|tiktok\.com|douyin\.com|x\.com|twitter\.com|instagram\.com|bandcamp\.com|mixcloud\.com|spotify\.com)\//i;

interface BrowserWatchCardProps {
  lang: UiLang;
  browserState: BrowserStatePayload;
  preview: BrowserPreviewPayload | null;
  isThinking?: boolean;
  mediaDownloadReady?: boolean;
  isBookmarked?: boolean;
  onToggleBookmark?: (url: string, title: string) => void;
  onSummarizePage?: (url: string, title: string) => void;
  onQuickDownloadUrl?: (url: string, format: string) => void;
}

/**
 * Compact live browser status & preview card docked inside ChatTab:
 * - While Buddy runs in the background, stays unobtrusive with a thumbnail + "Watch" CTA.
 * - Clicking "Watch" opens the companion Buddy Browser window and docks Buddy right beside it,
 *   revealing quick URL/nav controls, a "My Turn / Let Buddy Continue" takeover switch,
 *   and one-click Summarize / Media Download actions for the current page.
 */
export function BrowserWatchCard({
  lang,
  browserState,
  preview,
  isThinking = false,
  mediaDownloadReady = false,
  isBookmarked = false,
  onToggleBookmark,
  onSummarizePage,
  onQuickDownloadUrl,
}: BrowserWatchCardProps) {
  const s = getStrings(lang).chat;
  const [urlDraft, setUrlDraft] = useState(browserState.url);
  const [editingUrl, setEditingUrl] = useState(false);

  useEffect(() => {
    if (!editingUrl) {
      setUrlDraft(browserState.url);
    }
  }, [browserState.url, editingUrl]);

  if (!browserState.active) return null;

  const domain = (() => {
    try {
      const u = new URL(browserState.url);
      if (u.protocol === "file:") {
        const seg = decodeURIComponent(u.pathname).split("/").filter(Boolean).pop();
        return seg || "Local File";
      }
      return u.host || u.hostname;
    } catch {
      return browserState.url;
    }
  })();

  const handleNavigate = (e: React.FormEvent) => {
    e.preventDefault();
    const target = urlDraft.trim();
    if (!target) return;
    setEditingUrl(false);
    void invoke("browser_navigate", { url: target }).catch(() => {});
  };

  return (
    <div
      className={`mb-1.5 rounded-2xl border backdrop-blur-md transition-all shrink-0 overflow-hidden ${
        browserState.needs_attention
          ? "bg-amber-500/12 border-amber-400/35"
          : browserState.watching
          ? "bg-sky-500/10 border-sky-400/30"
          : "bg-white/[0.06] border-white/15"
      }`}
    >
      <div className="px-2.5 py-1.5 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          {!browserState.watching && preview?.data_url ? (
            <button
              type="button"
              onClick={() => void invoke("set_browser_watching", { watching: true }).catch(() => {})}
              title={s.browserWatchBtn}
              className="relative w-14 h-9 rounded-lg overflow-hidden border border-white/15 shrink-0 bg-black/50 group"
            >
              <img
                src={preview.data_url}
                alt=""
                className="w-full h-full object-cover group-hover:scale-105 transition-transform"
              />
              <span className="absolute inset-0 bg-black/25 opacity-0 group-hover:opacity-100 flex items-center justify-center transition-opacity">
                <Eye className="w-3.5 h-3.5 text-white" />
              </span>
            </button>
          ) : (
            <div className="w-7 h-7 rounded-xl bg-white/[0.08] border border-white/10 flex items-center justify-center shrink-0">
              <Globe
                className={`w-3.5 h-3.5 ${
                  browserState.navigating ? "text-sky-300 animate-pulse" : "text-sky-300"
                }`}
              />
            </div>
          )}

          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <span
                className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                  browserState.needs_attention
                    ? "bg-amber-400 animate-ping"
                    : browserState.user_takeover
                    ? "bg-amber-400"
                    : "bg-emerald-400 animate-pulse"
                }`}
              />
              <span className="text-[11px] font-medium text-white truncate">
                {browserState.title || domain || s.browserTitle}
              </span>
            </div>
            <div className="text-[10px] text-white/55 truncate">
              {browserState.action_label ||
                (browserState.watching ? s.browserWatching : s.browserBackground)}
              {domain ? ` · ${domain}` : ""}
            </div>
          </div>
        </div>

        <div className="flex items-center gap-1 shrink-0">
          <button
            type="button"
            onClick={() =>
              void invoke("set_browser_watching", { watching: !browserState.watching }).catch(() => {})
            }
            className={`px-2 py-1 rounded-full text-[10.5px] font-medium inline-flex items-center gap-1 border transition ${
              browserState.watching
                ? "bg-white/[0.12] hover:bg-white/[0.2] border-white/20 text-white"
                : "bg-sky-500/20 hover:bg-sky-500/35 border-sky-400/35 text-sky-200"
            }`}
          >
            {browserState.watching ? (
              <>
                <EyeOff className="w-3 h-3" />
                <span>{s.browserHideBtn}</span>
              </>
            ) : (
              <>
                <Eye className="w-3 h-3" />
                <span>{s.browserWatchBtn}</span>
              </>
            )}
          </button>

          <button
            type="button"
            title={s.browserCloseBtn}
            onClick={() => void invoke("close_builtin_browser").catch(() => {})}
            className="w-6 h-6 rounded-full bg-white/[0.06] hover:bg-rose-500/30 text-white/60 hover:text-rose-100 flex items-center justify-center transition"
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      </div>

      {!browserState.watching && browserState.needs_attention && (
        <div className="px-2.5 pb-2">
          <div className="text-[10px] text-amber-200 bg-amber-500/15 border border-amber-400/25 rounded-lg px-2 py-1 leading-snug">
            {s.browserAttentionHint}
          </div>
        </div>
      )}

      {browserState.watching && (
        <div className="px-2.5 pb-2 pt-1 border-t border-white/[0.07] space-y-1.5">
          {browserState.needs_attention && (
            <div className="text-[10px] text-amber-200 bg-amber-500/15 border border-amber-400/25 rounded-lg px-2 py-1 leading-snug">
              {s.browserAttentionHint}
            </div>
          )}
          <div className="flex items-center gap-1">
            <button
              type="button"
              title={s.browserBackTip}
              onClick={() => void invoke("browser_nav_action", { action: "back" }).catch(() => {})}
              className="p-1 rounded-lg bg-white/[0.06] hover:bg-white/[0.14] text-white/75 transition"
            >
              <ArrowLeft className="w-3 h-3" />
            </button>
            <button
              type="button"
              title={s.browserForwardTip}
              onClick={() =>
                void invoke("browser_nav_action", { action: "forward" }).catch(() => {})
              }
              className="p-1 rounded-lg bg-white/[0.06] hover:bg-white/[0.14] text-white/75 transition"
            >
              <ArrowRight className="w-3 h-3" />
            </button>
            <button
              type="button"
              title={s.browserReloadTip}
              onClick={() =>
                void invoke("browser_nav_action", { action: "reload" }).catch(() => {})
              }
              className="p-1 rounded-lg bg-white/[0.06] hover:bg-white/[0.14] text-white/75 transition"
            >
              <RotateCw className="w-3 h-3" />
            </button>

            <form onSubmit={handleNavigate} className="flex-1 min-w-0">
              <input
                type="text"
                value={urlDraft}
                onFocus={() => setEditingUrl(true)}
                onBlur={() => setEditingUrl(false)}
                onChange={(e) => setUrlDraft(e.target.value)}
                placeholder={s.browserUrlPlaceholder}
                className="w-full px-2 py-0.5 rounded-lg bg-black/35 border border-white/12 focus:border-sky-400/60 outline-none text-[10.5px] font-mono text-white/90"
              />
            </form>

            {onToggleBookmark && /^(?:https?|file):\/\//i.test(browserState.url) && (
              <button
                type="button"
                title={isBookmarked ? s.browserBookmarkedTip : s.browserBookmarkAddTip}
                onClick={() => onToggleBookmark(browserState.url, browserState.title || domain)}
                className={`p-1 rounded-lg border transition shrink-0 ${
                  isBookmarked
                    ? "bg-amber-500/20 border-amber-400/35 text-amber-300"
                    : "bg-white/[0.06] border-transparent hover:bg-white/[0.14] text-white/60 hover:text-amber-200"
                }`}
              >
                <Star className={`w-3 h-3 ${isBookmarked ? "fill-amber-300" : ""}`} />
              </button>
            )}

            <button
              type="button"
              onClick={() =>
                void invoke("set_browser_takeover", {
                  takeover: !browserState.user_takeover,
                }).catch(() => {})
              }
              className={`shrink-0 px-2 py-1 rounded-lg text-[10px] font-medium inline-flex items-center gap-1 border transition ${
                browserState.user_takeover
                  ? "bg-emerald-500/25 hover:bg-emerald-500/35 border-emerald-400/40 text-emerald-100"
                  : "bg-amber-500/15 hover:bg-amber-500/25 border-amber-400/30 text-amber-200"
              }`}
            >
              {browserState.user_takeover ? (
                <>
                  <Play className="w-2.5 h-2.5 fill-current" />
                  <span>{s.browserResumeAi}</span>
                </>
              ) : (
                <>
                  <Hand className="w-2.5 h-2.5" />
                  <span>{s.browserTakeControl}</span>
                </>
              )}
            </button>
          </div>

          {/^(?:https?|file):\/\//i.test(browserState.url) && (onSummarizePage || onQuickDownloadUrl) && (
            <div className="flex items-center gap-1.5 flex-wrap pt-0.5">
              {onSummarizePage && (
                <button
                  type="button"
                  disabled={isThinking}
                  onClick={() => onSummarizePage(browserState.url, browserState.title || domain)}
                  className="px-2 py-0.5 rounded-full bg-sky-500/15 hover:bg-sky-500/30 border border-sky-400/25 disabled:opacity-40 text-[10px] text-sky-200 inline-flex items-center gap-1 transition"
                >
                  <Sparkles className="w-2.5 h-2.5" />
                  <span>{s.browserSummarizePage}</span>
                </button>
              )}
              {mediaDownloadReady &&
                onQuickDownloadUrl &&
                BROWSER_MEDIA_URL_RE.test(browserState.url) && (
                  <>
                    <button
                      type="button"
                      onClick={() => onQuickDownloadUrl(browserState.url, "audio_mp3")}
                      className="px-2 py-0.5 rounded-full bg-emerald-500/15 hover:bg-emerald-500/30 border border-emerald-400/25 text-[10px] text-emerald-200 inline-flex items-center gap-1 transition"
                    >
                      <Music className="w-2.5 h-2.5" />
                      <span>{s.browserDownloadAudio}</span>
                    </button>
                    <button
                      type="button"
                      onClick={() => onQuickDownloadUrl(browserState.url, "mp4_lossless")}
                      className="px-2 py-0.5 rounded-full bg-emerald-500/15 hover:bg-emerald-500/30 border border-emerald-400/25 text-[10px] text-emerald-200 inline-flex items-center gap-1 transition"
                    >
                      <Download className="w-2.5 h-2.5" />
                      <span>{s.browserDownloadVideo}</span>
                    </button>
                  </>
                )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
