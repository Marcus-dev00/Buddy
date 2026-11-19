import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import QRCode from "qrcode";
import {
  Check,
  Cloud,
  Copy,
  Download,
  ExternalLink,
  Loader2,
  QrCode,
  RefreshCw,
  Shield,
  Smartphone,
  Wifi,
} from "lucide-react";
import { ToggleSwitch } from "./SettingsControls";
import type { MobileBridgeStatus, UiLang } from "../types";

const DEFAULT_DOWNLOAD_SITE = "https://buddy-companion.marcus-dev00.workers.dev";

export function MobileLinkCard({
  lang,
  isWindowMode = false,
}: {
  lang: UiLang;
  isWindowMode?: boolean;
}) {
  const isZh = lang === "zh";
  const [st, setSt] = useState<MobileBridgeStatus | null>(null);
  const [qrTab, setQrTab] = useState<"download" | "pair">("pair");
  const [qrDataUrl, setQrDataUrl] = useState<string>("");
  const [dlQrDataUrl, setDlQrDataUrl] = useState<string>("");
  const [customRemote, setCustomRemote] = useState("");
  const [dirUrl, setDirUrl] = useState("");
  const [autoTunnel, setAutoTunnel] = useState(false);
  const [busyTunnel, setBusyTunnel] = useState(false);
  const [copiedPair, setCopiedPair] = useState(false);
  const [copiedSite, setCopiedSite] = useState(false);
  const [savedNote, setSavedNote] = useState(false);
  const [errMsg, setErrMsg] = useState<string | null>(null);

  const downloadSiteUrl = (st?.directory_url || dirUrl || DEFAULT_DOWNLOAD_SITE)
    .trim()
    .replace(/\/$/, "");

  const syncFromStatus = (next: MobileBridgeStatus) => {
    setSt(next);
    setCustomRemote(next.custom_remote_url);
    setDirUrl(next.directory_url);
    setAutoTunnel(Boolean(next.tunnel_running));
    if (!next.running) {
      setQrTab("download");
    }
  };

  useEffect(() => {
    void invoke<MobileBridgeStatus>("get_mobile_bridge_status")
      .then(syncFromStatus)
      .catch(() => {});
    const unlisten = listen<MobileBridgeStatus>("buddy://mobile-bridge-status", (ev) => {
      if (ev.payload) {
        setSt(ev.payload);
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void QRCode.toDataURL(downloadSiteUrl, {
      width: 176,
      margin: 1,
      errorCorrectionLevel: "M",
      color: { dark: "#000000", light: "#ffffff" },
    })
      .then((url) => {
        if (!cancelled) setDlQrDataUrl(url);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [downloadSiteUrl]);

  useEffect(() => {
    if (!st?.pairing_payload || !st.running) {
      setQrDataUrl("");
      return;
    }
    let cancelled = false;
    void QRCode.toDataURL(st.pairing_payload, {
      width: 176,
      margin: 1,
      errorCorrectionLevel: "M",
      color: { dark: "#000000", light: "#ffffff" },
    })
      .then((url) => {
        if (!cancelled) setQrDataUrl(url);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [st?.pairing_payload, st?.running]);

  const handleToggleBridge = async (enabled: boolean) => {
    setErrMsg(null);
    try {
      const next = await invoke<MobileBridgeStatus>("set_mobile_bridge_enabled", { enabled });
      setSt(next);
      if (next.running) setQrTab("pair");
    } catch (e) {
      setErrMsg(String(e));
    }
  };

  const handleToggleTunnel = async () => {
    if (!st || busyTunnel) return;
    setErrMsg(null);
    setBusyTunnel(true);
    try {
      const next = st.tunnel_running
        ? await invoke<MobileBridgeStatus>("stop_mobile_tunnel")
        : await invoke<MobileBridgeStatus>("start_mobile_tunnel");
      setSt(next);
    } catch (e) {
      setErrMsg(String(e));
    } finally {
      setBusyTunnel(false);
    }
  };

  const handleSaveRemote = async () => {
    setErrMsg(null);
    try {
      const next = await invoke<MobileBridgeStatus>("update_mobile_remote_settings", {
        customRemoteUrl: customRemote,
        directoryUrl: dirUrl,
        autoTunnel,
      });
      setSt(next);
      setSavedNote(true);
      window.setTimeout(() => setSavedNote(false), 1400);
    } catch (e) {
      setErrMsg(String(e));
    }
  };

  const handleResetToken = async () => {
    setErrMsg(null);
    try {
      const next = await invoke<MobileBridgeStatus>("reset_mobile_pairing_token");
      setSt(next);
    } catch (e) {
      setErrMsg(String(e));
    }
  };

  return (
    <div className="flex-1 flex flex-col min-h-0 gap-2 overflow-y-auto pr-0.5">
      {/* Main Bridge Toggle Card */}
      <div className="rounded-2xl bg-white/[0.045] border border-white/10 p-2.5 space-y-2">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2 min-w-0">
            <div className="w-7 h-7 rounded-xl bg-sky-500/15 border border-sky-400/25 flex items-center justify-center shrink-0">
              <Smartphone className="w-3.5 h-3.5 text-sky-300" />
            </div>
            <div className="min-w-0">
              <div className="text-[11.5px] font-semibold text-white flex items-center gap-1.5">
                <span>{isZh ? "手机伴侣桥接" : "Mobile Companion Bridge"}</span>
                {st?.running && (
                  <span className="px-1.5 py-[1px] rounded-full bg-emerald-500/15 border border-emerald-400/25 text-[9px] text-emerald-300">
                    {st.connected_clients > 0
                      ? isZh
                        ? `${st.connected_clients} 台已连`
                        : `${st.connected_clients} online`
                      : isZh
                      ? "运行中"
                      : "Live"}
                  </span>
                )}
              </div>
              <div className="text-[10px] text-white/45 truncate">
                {isZh
                  ? "用 BuddyMobile APK 或网页版扫码远程连接此电脑"
                  : "Scan QR from BuddyMobile APK or web app to control this PC"}
              </div>
            </div>
          </div>
          <ToggleSwitch
            checked={Boolean(st?.enabled)}
            onChange={(v) => void handleToggleBridge(v)}
          />
        </div>

        {/* Remote Cloudflare Quick Tunnel */}
        {st?.running && (
          <div className="pt-2 border-t border-white/[0.07] space-y-1.5">
            <div className="flex items-center justify-between gap-2">
              <div className="min-w-0">
                <div className="text-[11px] font-medium text-white flex items-center gap-1.5">
                  <Cloud className="w-3 h-3 text-amber-300" />
                  <span>
                    {isZh ? "出门 5G 免费远程隧道 (Cloudflare)" : "Remote 5G Tunnel (Cloudflare)"}
                  </span>
                </div>
                <div className="text-[9.5px] text-white/45 truncate font-mono">
                  {st.tunnel_downloading
                    ? `${isZh ? "正在下载 cloudflared..." : "Downloading cloudflared..."} ${st.tunnel_progress}%`
                    : st.tunnel_url
                    ? st.tunnel_url
                    : st.tailscale_url
                    ? `Tailscale: ${st.tailscale_url}`
                    : isZh
                    ? "按需一键拉起免费加密 HTTPS 隧道，手机免装组网软件"
                    : "One-click free HTTPS tunnel — no VPN app needed on phone"}
                </div>
              </div>
              <button
                type="button"
                disabled={busyTunnel || st.tunnel_downloading}
                onClick={() => void handleToggleTunnel()}
                className={`px-2.5 py-1 rounded-full text-[10px] font-semibold inline-flex items-center gap-1 shrink-0 transition ${
                  st.tunnel_running
                    ? "bg-rose-500/20 hover:bg-rose-500/35 border border-rose-400/30 text-rose-200"
                    : "bg-amber-400 hover:bg-amber-300 text-slate-950"
                }`}
              >
                {busyTunnel || st.tunnel_downloading ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <Cloud className="w-3 h-3" />
                )}
                <span>
                  {st.tunnel_running
                    ? isZh
                      ? "关闭隧道"
                      : "Stop"
                    : isZh
                    ? "开启远程"
                    : "Start"}
                </span>
              </button>
            </div>

            {st.lan_url && (
              <div className="flex items-center gap-1.5 text-[9.5px] font-mono text-white/50">
                <Wifi className="w-2.5 h-2.5 text-emerald-300 shrink-0" />
                <span className="truncate">LAN: {st.lan_url}</span>
              </div>
            )}
          </div>
        )}
      </div>

      {(errMsg || st?.last_error) && (
        <div className="rounded-xl bg-rose-500/15 border border-rose-400/30 px-2.5 py-1.5 text-[10px] text-rose-200 break-words">
          {errMsg || st?.last_error}
        </div>
      )}

      {/* 2-Step QR Card: ① Download App / Web | ② Scan to Pair */}
      <div className="rounded-2xl bg-white/[0.045] border border-white/10 p-3 space-y-2.5">
        <div className="grid grid-cols-2 gap-1 p-0.5 rounded-full bg-black/45 border border-white/10">
          <button
            type="button"
            onClick={() => setQrTab("download")}
            className={`py-1 px-2 rounded-full text-[10.5px] font-medium inline-flex items-center justify-center gap-1 transition ${
              qrTab === "download"
                ? "bg-white text-black font-semibold"
                : "text-white/55 hover:text-white"
            }`}
          >
            <Download className="w-3 h-3" />
            <span>{isZh ? "① 下载手机端" : "① Get Mobile App"}</span>
          </button>
          <button
            type="button"
            onClick={() => setQrTab("pair")}
            className={`py-1 px-2 rounded-full text-[10.5px] font-medium inline-flex items-center justify-center gap-1 transition ${
              qrTab === "pair"
                ? "bg-white text-black font-semibold"
                : "text-white/55 hover:text-white"
            }`}
          >
            <QrCode className="w-3 h-3" />
            <span>{isZh ? "② 扫码配对" : "② Scan to Pair"}</span>
          </button>
        </div>

        {qrTab === "download" ? (
          <div className="flex items-center gap-3">
            {dlQrDataUrl ? (
              <div className="p-2 rounded-xl bg-white shrink-0 shadow-lg">
                <img
                  src={dlQrDataUrl}
                  alt="Download BuddyMobile QR"
                  className={isWindowMode ? "w-[144px] h-[144px] block" : "w-[106px] h-[106px] block"}
                />
              </div>
            ) : (
              <div
                className={`rounded-xl bg-white/10 flex items-center justify-center shrink-0 ${
                  isWindowMode ? "w-[160px] h-[160px]" : "w-[118px] h-[118px]"
                }`}
              >
                <Loader2 className="w-4 h-4 animate-spin text-white/50" />
              </div>
            )}

            <div className="min-w-0 flex-1 space-y-2">
              <div>
                <div className="text-[11.5px] font-semibold text-white">
                  {isZh ? "手机扫码下载 APK 或直开网页版" : "Scan to Get APK or Mobile Web"}
                </div>
                <p className="text-[10px] text-white/50 leading-relaxed mt-0.5">
                  {isZh
                    ? "首次使用请先用手机相机或微信扫左侧二维码，直接下载安装 BuddyMobile.apk，装好后切到「② 扫码配对」连接。"
                    : "Scan with your phone camera to download BuddyMobile.apk or open the web app, then switch to Step ② to pair."}
                </p>
              </div>
              <div className="flex items-center flex-wrap gap-1.5">
                <button
                  type="button"
                  onClick={() => {
                    void navigator.clipboard.writeText(downloadSiteUrl).then(() => {
                      setCopiedSite(true);
                      window.setTimeout(() => setCopiedSite(false), 1500);
                    });
                  }}
                  className="px-2.5 py-1 rounded-full bg-white text-black text-[10px] font-semibold inline-flex items-center gap-1 transition"
                >
                  {copiedSite ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
                  <span>
                    {copiedSite
                      ? isZh
                        ? "已复制网址"
                        : "Copied"
                      : isZh
                      ? "复制下载网址"
                      : "Copy Link"}
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => {
                    void invoke("open_external_url", { url: downloadSiteUrl }).catch(() => {});
                  }}
                  className="px-2.5 py-1 rounded-full bg-white/[0.08] hover:bg-white/[0.15] border border-white/12 text-[10px] text-white/85 inline-flex items-center gap-1 transition"
                >
                  <ExternalLink className="w-2.5 h-2.5" />
                  <span>{isZh ? "浏览器打开" : "Open Site"}</span>
                </button>
              </div>
            </div>
          </div>
        ) : st?.running ? (
          <div className="flex items-center gap-3">
            {qrDataUrl ? (
              <div className="p-2 rounded-xl bg-white shrink-0 shadow-lg">
                <img
                  src={qrDataUrl}
                  alt="Pairing QR"
                  className={isWindowMode ? "w-[144px] h-[144px] block" : "w-[106px] h-[106px] block"}
                />
              </div>
            ) : (
              <div
                className={`rounded-xl bg-white/10 flex items-center justify-center shrink-0 ${
                  isWindowMode ? "w-[160px] h-[160px]" : "w-[118px] h-[118px]"
                }`}
              >
                <Loader2 className="w-4 h-4 animate-spin text-white/50" />
              </div>
            )}

            <div className="min-w-0 flex-1 space-y-2">
              <div>
                <div className="text-[11.5px] font-semibold text-white">
                  {isZh ? "打开 BuddyMobile 扫码配对" : "Scan with BuddyMobile"}
                </div>
                <p className="text-[10px] text-white/50 leading-relaxed mt-0.5">
                  {isZh
                    ? "用手机上的 BuddyMobile 扫左侧二维码，或点下方复制配对码粘贴即可加密连接。"
                    : "Scan with BuddyMobile App or copy the pairing code below to connect."}
                </p>
              </div>
              <div className="flex items-center flex-wrap gap-1.5">
                <button
                  type="button"
                  onClick={() => {
                    void navigator.clipboard.writeText(st.pairing_payload).then(() => {
                      setCopiedPair(true);
                      window.setTimeout(() => setCopiedPair(false), 1500);
                    });
                  }}
                  className="px-2.5 py-1 rounded-full bg-white text-black text-[10px] font-semibold inline-flex items-center gap-1 transition"
                >
                  {copiedPair ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
                  <span>
                    {copiedPair
                      ? isZh
                        ? "已复制配对码"
                        : "Copied"
                      : isZh
                      ? "复制配对码"
                      : "Copy Pair Code"}
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => void handleResetToken()}
                  title={isZh ? "重置后旧手机需重新扫码" : "Revoke old paired phones"}
                  className="px-2 py-1 rounded-full bg-white/[0.06] hover:bg-rose-500/20 border border-white/10 text-[10px] text-white/65 hover:text-rose-200 inline-flex items-center gap-1 transition"
                >
                  <RefreshCw className="w-2.5 h-2.5" />
                  <span>{isZh ? "重置密钥" : "Reset Key"}</span>
                </button>
              </div>
            </div>
          </div>
        ) : (
          <div className="py-5 text-center space-y-2">
            <div className="text-[11px] text-white/55">
              {isZh ? "请先打开上方「手机伴侣桥接」开关以生成配对二维码" : "Enable Mobile Companion Bridge above to generate your pairing QR"}
            </div>
            <button
              type="button"
              onClick={() => void handleToggleBridge(true)}
              className="px-3 py-1 rounded-full bg-white text-black text-[10.5px] font-semibold"
            >
              {isZh ? "立即开启桥接" : "Enable Bridge"}
            </button>
          </div>
        )}
      </div>

      {/* Advanced Remote / Tailscale & Directory */}
      <div className="rounded-2xl bg-white/[0.035] border border-white/10 p-2.5 space-y-2">
        <div className="flex items-center gap-1.5 text-[10.5px] font-medium text-white/75">
          <Shield className="w-3 h-3 text-sky-300" />
          <span>{isZh ? "固定远程地址（Tailscale / 自定义域名）与动态目录" : "Fixed Remote URL & Dynamic Directory"}</span>
        </div>
        <input
          type="text"
          value={customRemote}
          onChange={(e) => setCustomRemote(e.target.value)}
          placeholder={
            st?.tailscale_url
              ? `Tailscale (${st.tailscale_url})`
              : isZh
              ? "可选：Tailscale 地址如 http://100.x.x.x:19425 或自定义域名"
              : "Optional: Tailscale http://100.x.x.x:19425 or custom HTTPS URL"
          }
          className="w-full rounded-xl bg-black/35 border border-white/10 focus:border-sky-400/50 outline-none px-2.5 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30"
        />
        <input
          type="text"
          value={dirUrl}
          onChange={(e) => setDirUrl(e.target.value)}
          placeholder="https://buddy-companion.<subdomain>.workers.dev"
          className="w-full rounded-xl bg-black/35 border border-white/10 focus:border-sky-400/50 outline-none px-2.5 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30"
        />
        <div className="flex items-center justify-between gap-2">
          <label className="flex items-center gap-1.5 text-[10px] text-white/65 cursor-pointer select-none">
            <input
              type="checkbox"
              checked={autoTunnel}
              onChange={(e) => setAutoTunnel(e.target.checked)}
              className="accent-sky-400 rounded"
            />
            <span>{isZh ? "启动 Buddy 时自动开启远程隧道" : "Auto-start remote tunnel on launch"}</span>
          </label>
          <button
            type="button"
            onClick={() => void handleSaveRemote()}
            className="px-2.5 py-1 rounded-full bg-white/[0.1] hover:bg-white/[0.18] text-[10px] font-medium text-white transition"
          >
            {savedNote ? (isZh ? "已保存" : "Saved") : isZh ? "保存" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
