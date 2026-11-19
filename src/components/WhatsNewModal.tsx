import { useEffect } from "react";
import { Maximize2, Plug, Shield, Smartphone, Sparkles, X } from "lucide-react";
import type { UiLang } from "../types";

const LAST_SEEN_VERSION_KEY = "buddy_last_seen_version";

export interface ReleaseHighlight {
  icon: "window" | "mobile" | "mcp" | "shield" | "spark";
  titleZh: string;
  descZh: string;
  titleEn: string;
  descEn: string;
}

export interface ReleaseEntry {
  version: string;
  items: ReleaseHighlight[];
}

/**
 * Concise human-friendly highlights shown once on first expand after an upgrade.
 * Keep the newest release at index 0.
 */
export const RELEASE_NOTES: ReleaseEntry[] = [
  {
    version: "0.3.1",
    items: [
      {
        icon: "spark",
        titleZh: "版本更新亮点提醒卡片",
        descZh: "升级后首次展开 Buddy 自动展示本次更新核心变化（全新安装不打扰），也可随时在「设置 → 通用 → 更新亮点」回看。",
        titleEn: "Post-Update What's New Card",
        descEn: "After upgrading, Buddy shows a one-time summary of new features the first time you expand it—and you can revisit it anytime in Settings → General.",
      },
      {
        icon: "window",
        titleZh: "大窗 / 全屏模式自由切换",
        descZh: "点击右上角放大按钮切入居中大窗或全屏，左侧常驻历史边栏并支持任务栏与 Alt+Tab 切回。",
        titleEn: "Full Window & Fullscreen Mode",
        descEn: "Switch between the compact Dynamic Island and a spacious desktop window with a permanent history sidebar and Taskbar support.",
      },
      {
        icon: "mobile",
        titleZh: "BuddyMobile 手机伴侣 & 实时同步",
        descZh: "前往「设置 → 手机」先扫 ① 下载 APK 或打开在线手机端，再扫 ② 配对，手机聊天记录实时同步至电脑并带来源图标区分。",
        titleEn: "BuddyMobile & Live Sync",
        descEn: "Scan Step ① in Settings → Mobile for the Android APK or Mobile Web App, then Step ② to pair with live cross-device chat sync.",
      },
      {
        icon: "mcp",
        titleZh: "MCP 独立设置入口 & 对话页瘦身",
        descZh: "MCP 外部工具管理已移至「设置 → MCP」；权限模式位于对话顶栏下拉菜单，思考等级收至「设置 → 通用」。",
        titleEn: "Dedicated MCP Settings & Cleaner Chat",
        descEn: "Manage stdio and HTTP MCP servers in Settings → MCP, switch permission modes from the chat top bar, and adjust thinking effort in Settings → General.",
      },
    ],
  },
  {
    version: "0.3.0",
    items: [
      {
        icon: "window",
        titleZh: "全新大窗 / 全屏模式",
        descZh: "点击右上角放大按钮即可切入居中大窗或全屏（替代原 Pin 按钮），左侧常驻历史边栏并支持任务栏与 Alt+Tab 切回。",
        titleEn: "Full Window & Fullscreen Mode",
        descEn: "Click expand to switch from the Dynamic Island into a spacious desktop window (replacing Pin) with a permanent history sidebar and Taskbar support.",
      },
      {
        icon: "mobile",
        titleZh: "BuddyMobile 手机伴侣",
        descZh: "前往「设置 → 手机」先扫 ① 下载 APK 或直开在线手机端，再扫 ② 配对，出门在外也能随时遥控电脑 Buddy 与查看 7 天记忆。",
        titleEn: "BuddyMobile Companion",
        descEn: "Open Settings → Mobile to scan Step ① for the Android APK / Mobile Web App, then Step ② to pair and control your PC from anywhere.",
      },
      {
        icon: "mcp",
        titleZh: "MCP 服务入口移至「设置」",
        descZh: "支持 stdio 与 HTTP MCP 协议接入外部工具；现已独立放在「设置 → MCP」，查找与管理更清晰。",
        titleEn: "MCP Hub Moved to Settings",
        descEn: "Connect stdio and HTTP MCP servers directly under Settings → MCP with JSON import and per-server trust controls.",
      },
      {
        icon: "shield",
        titleZh: "对话页瘦身 & 实时多端同步",
        descZh: "权限模式移至对话顶栏一键下拉切换，思考等级收至「设置 → 通用」；手机端聊天记录实时同步至电脑并带来源图标区分。",
        titleEn: "Cleaner Chat & Live Cross-Device Sync",
        descEn: "Permission switcher now sits neatly in the chat top bar, thinking effort lives in Settings → General, and mobile chats sync live with source icons.",
      },
    ],
  },
];

export function resolveReleaseEntry(version: string): ReleaseEntry {
  const exact = RELEASE_NOTES.find((r) => r.version === version);
  if (exact) return exact;
  return { ...RELEASE_NOTES[0], version };
}

/**
 * Returns true ONLY when an existing user has upgraded to a newer version they haven't seen yet.
 * Brand-new installs silently record the current version and return false.
 */
export function checkShouldShowWhatsNew(appVersion: string, hasPriorUsage: boolean): boolean {
  if (!appVersion || appVersion === "0.1.0") return false;
  try {
    const lastSeen = localStorage.getItem(LAST_SEEN_VERSION_KEY);
    if (!lastSeen) {
      if (hasPriorUsage) {
        return true;
      }
      localStorage.setItem(LAST_SEEN_VERSION_KEY, appVersion);
      return false;
    }
    return lastSeen !== appVersion;
  } catch {
    return false;
  }
}

export function markWhatsNewSeen(appVersion: string): void {
  if (!appVersion) return;
  try {
    localStorage.setItem(LAST_SEEN_VERSION_KEY, appVersion);
  } catch {
    // ignore storage errors
  }
}

function HighlightIcon({ kind }: { kind: ReleaseHighlight["icon"] }) {
  const cls = "w-3.5 h-3.5 text-white/85 shrink-0";
  switch (kind) {
    case "window":
      return <Maximize2 className={cls} />;
    case "mobile":
      return <Smartphone className={cls} />;
    case "mcp":
      return <Plug className={cls} />;
    case "shield":
      return <Shield className={cls} />;
    default:
      return <Sparkles className={cls} />;
  }
}

export interface WhatsNewModalProps {
  version: string;
  lang: UiLang;
  isWindowMode: boolean;
  onClose: () => void;
}

export function WhatsNewModal({
  version,
  lang,
  isWindowMode,
  onClose,
}: WhatsNewModalProps) {
  const isZh = lang === "zh";
  const entry = resolveReleaseEntry(version);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div
      onClick={onClose}
      className={
        isWindowMode
          ? "fixed inset-0 z-50 bg-black/80 backdrop-blur-md flex items-center justify-center p-6"
          : "absolute inset-0 z-40 rounded-2xl bg-black/88 backdrop-blur-md flex items-center justify-center p-2.5"
      }
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className={`buddy-window-enter w-full bg-[#0a0a0e] border border-white/[0.14] shadow-2xl flex flex-col overflow-hidden ${
          isWindowMode
            ? "max-w-lg rounded-3xl p-6 space-y-5"
            : "max-h-full rounded-2xl p-3.5 space-y-3"
        }`}
      >
        {/* Header */}
        <div className="flex items-start justify-between gap-3 shrink-0">
          <div className="flex items-center gap-2.5 min-w-0">
            <div className="flex items-center gap-1 px-1 shrink-0" aria-hidden="true">
              <span className="w-[5px] h-[12px] rounded-full bg-white" />
              <span className="w-[5px] h-[12px] rounded-full bg-white" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h3
                  className={`font-semibold text-white tracking-tight ${
                    isWindowMode ? "text-[16px]" : "text-[13px]"
                  }`}
                >
                  {isZh ? "Buddy 已更新" : "What's New in Buddy"}
                </h3>
                <span className="px-2 py-0.5 rounded-full bg-white/[0.1] border border-white/[0.15] font-mono text-[10px] text-white/90">
                  v{entry.version}
                </span>
              </div>
              <p className="text-[10.5px] text-white/45 mt-0.5">
                {isZh ? "本次版本的主要新功能与变化" : "Key highlights in this release"}
              </p>
            </div>
          </div>

          <button
            type="button"
            onClick={onClose}
            aria-label={isZh ? "关闭" : "Close"}
            className="w-7 h-7 rounded-full bg-white/[0.07] hover:bg-white/[0.16] text-white/65 hover:text-white flex items-center justify-center shrink-0 transition"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>

        {/* Highlights List */}
        <div className="flex-1 min-h-0 overflow-y-auto space-y-2 pr-0.5">
          {entry.items.map((item, idx) => (
            <div
              key={idx}
              className={`rounded-2xl bg-white/[0.04] border border-white/[0.08] flex items-start gap-2.5 ${
                isWindowMode ? "p-3.5" : "p-2.5"
              }`}
            >
              <div className="w-7 h-7 rounded-xl bg-white/[0.08] border border-white/[0.1] flex items-center justify-center shrink-0 mt-0.5">
                <HighlightIcon kind={item.icon} />
              </div>
              <div className="min-w-0 flex-1">
                <div
                  className={`font-medium text-white ${
                    isWindowMode ? "text-[13px]" : "text-[11.5px]"
                  }`}
                >
                  {isZh ? item.titleZh : item.titleEn}
                </div>
                <p
                  className={`text-white/55 leading-relaxed mt-0.5 ${
                    isWindowMode ? "text-[12px]" : "text-[10.5px]"
                  }`}
                >
                  {isZh ? item.descZh : item.descEn}
                </p>
              </div>
            </div>
          ))}
        </div>

        {/* Dismiss CTA */}
        <button
          type="button"
          onClick={onClose}
          className={`w-full rounded-full bg-white hover:bg-white/90 text-black font-semibold shrink-0 transition ${
            isWindowMode ? "py-2.5 text-[13px]" : "py-2 text-[11.5px]"
          }`}
        >
          {isZh ? "知道了，开始使用" : "Got it"}
        </button>
      </div>
    </div>
  );
}
