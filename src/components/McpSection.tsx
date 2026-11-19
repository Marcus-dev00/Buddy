import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronDown,
  Code2,
  Info,
  Loader2,
  Plug,
  Plus,
  RefreshCw,
  ShieldCheck,
  Terminal,
  Trash2,
  Globe,
  Wrench,
} from "lucide-react";
import { getStrings } from "../i18n";
import type { McpServerConfig, McpServerStatus, UiLang } from "../types";

export interface McpSectionProps {
  lang: UiLang;
  servers: McpServerStatus[];
  onRefreshServers: (next: McpServerStatus[]) => void;
  onError: (msg: string | null) => void;
}

function parseKvLines(raw: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of raw.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const eq = trimmed.indexOf("=");
    if (eq > 0) {
      const k = trimmed.slice(0, eq).trim();
      const v = trimmed.slice(eq + 1).trim();
      if (k) out[k] = v;
    }
  }
  return out;
}

export function McpSection({
  lang,
  servers,
  onRefreshServers,
  onError,
}: McpSectionProps) {
  const s = getStrings(lang).plugins;
  const [mode, setMode] = useState<"none" | "form" | "json">("none");
  const [busy, setBusy] = useState(false);
  const [openId, setOpenId] = useState<string | null>(null);

  // Form fields
  const [transport, setTransport] = useState<"stdio" | "http">("stdio");
  const [name, setName] = useState("");
  const [command, setCommand] = useState("npx");
  const [argsStr, setArgsStr] = useState("");
  const [envStr, setEnvStr] = useState("");
  const [url, setUrl] = useState("");
  const [headersStr, setHeadersStr] = useState("");

  // JSON import field
  const [jsonDraft, setJsonDraft] = useState("");

  useEffect(() => {
    void invoke<McpServerStatus[]>("list_mcp_servers")
      .then(onRefreshServers)
      .catch(() => {});
  }, []);

  const handleSaveForm = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || busy) return;
    onError(null);
    setBusy(true);
    try {
      const args = argsStr
        .trim()
        .split(/\s+/)
        .filter((x) => x.length > 0);
      const config: McpServerConfig = {
        id: "",
        name: name.trim(),
        transport,
        command: transport === "stdio" ? command.trim() : "",
        args: transport === "stdio" ? args : [],
        env: transport === "stdio" ? parseKvLines(envStr) : {},
        url: transport === "http" ? url.trim() : "",
        headers: transport === "http" ? parseKvLines(headersStr) : {},
        enabled: true,
        trusted: false,
      };
      const next = await invoke<McpServerStatus[]>("save_mcp_server", { config });
      onRefreshServers(next);
      setName("");
      setArgsStr("");
      setEnvStr("");
      setUrl("");
      setHeadersStr("");
      setMode("none");
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleImportJson = async () => {
    if (!jsonDraft.trim() || busy) return;
    onError(null);
    setBusy(true);
    try {
      const next = await invoke<McpServerStatus[]>("import_mcp_servers_json", {
        rawJson: jsonDraft,
      });
      onRefreshServers(next);
      setJsonDraft("");
      setMode("none");
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex-1 flex flex-col min-h-0 space-y-2">
      {/* Explainer banner */}
      <div className="rounded-xl bg-violet-500/[0.07] border border-violet-400/20 px-2.5 py-1.5 flex items-start gap-2 shrink-0">
        <Info className="w-3.5 h-3.5 text-violet-300/85 shrink-0 mt-0.5" />
        <p className="text-[10px] text-violet-100/80 leading-relaxed">{s.mcpExplainBanner}</p>
      </div>

      {/* Action Buttons Row */}
      <div className="flex items-center justify-between gap-1.5 shrink-0">
        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={() => setMode((m) => (m === "form" ? "none" : "form"))}
            className={`px-2.5 py-1 rounded-full border text-[10.5px] font-medium inline-flex items-center gap-1 transition ${
              mode === "form"
                ? "bg-violet-500/25 border-violet-400/40 text-violet-100"
                : "bg-white/[0.06] hover:bg-white/[0.12] border-white/10 text-white/85"
            }`}
          >
            <Plus className="w-3 h-3" />
            <span>{s.mcpAddBtn}</span>
          </button>
          <button
            type="button"
            onClick={() => setMode((m) => (m === "json" ? "none" : "json"))}
            className={`px-2.5 py-1 rounded-full border text-[10.5px] font-medium inline-flex items-center gap-1 transition ${
              mode === "json"
                ? "bg-violet-500/25 border-violet-400/40 text-violet-100"
                : "bg-white/[0.06] hover:bg-white/[0.12] border-white/10 text-white/85"
            }`}
          >
            <Code2 className="w-3 h-3" />
            <span>{s.mcpImportJsonBtn}</span>
          </button>
        </div>

        {servers.length > 0 && (
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              onError(null);
              setBusy(true);
              void invoke<McpServerStatus[]>("reconnect_all_mcp_servers")
                .then(onRefreshServers)
                .catch((e) => onError(String(e)))
                .finally(() => setBusy(false));
            }}
            className="px-2 py-1 rounded-full bg-white/[0.05] hover:bg-white/[0.12] border border-white/10 disabled:opacity-40 text-[10px] text-white/75 inline-flex items-center gap-1 transition"
          >
            <RefreshCw className={`w-2.5 h-2.5 ${busy ? "animate-spin" : ""}`} />
            <span>{s.mcpReconnectAll}</span>
          </button>
        )}
      </div>

      {/* Add Server Form */}
      {mode === "form" && (
        <form
          onSubmit={(e) => void handleSaveForm(e)}
          className="rounded-2xl bg-white/[0.045] border border-violet-400/25 p-2.5 space-y-2 shrink-0"
        >
          <div className="flex items-center gap-1.5">
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={s.mcpNamePlaceholder}
              className="flex-1 min-w-0 rounded-xl bg-black/40 border border-white/12 focus:border-violet-400/50 outline-none px-2.5 py-1 text-[11px] text-white placeholder:text-white/30"
            />
            <div className="flex items-center rounded-xl bg-black/40 border border-white/10 p-0.5 shrink-0">
              {(["stdio", "http"] as const).map((tr) => (
                <button
                  key={tr}
                  type="button"
                  onClick={() => setTransport(tr)}
                  className={`px-2 py-0.5 rounded-lg text-[10px] font-mono uppercase transition ${
                    transport === tr
                      ? "bg-violet-500/30 text-violet-100"
                      : "text-white/45 hover:text-white/80"
                  }`}
                >
                  {tr}
                </button>
              ))}
            </div>
          </div>

          {transport === "stdio" ? (
            <>
              <div className="flex items-center gap-1.5">
                <input
                  type="text"
                  value={command}
                  onChange={(e) => setCommand(e.target.value)}
                  placeholder={s.mcpCmdPlaceholder}
                  className="w-24 rounded-xl bg-black/40 border border-white/12 focus:border-violet-400/50 outline-none px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30"
                />
                <input
                  type="text"
                  value={argsStr}
                  onChange={(e) => setArgsStr(e.target.value)}
                  placeholder={s.mcpArgsPlaceholder}
                  className="flex-1 min-w-0 rounded-xl bg-black/40 border border-white/12 focus:border-violet-400/50 outline-none px-2 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30"
                />
              </div>
              <textarea
                rows={2}
                value={envStr}
                onChange={(e) => setEnvStr(e.target.value)}
                placeholder={s.mcpEnvPlaceholder}
                className="w-full rounded-xl bg-black/40 border border-white/12 focus:border-violet-400/50 outline-none px-2.5 py-1 text-[10px] font-mono text-white placeholder:text-white/30 resize-none"
              />
            </>
          ) : (
            <>
              <input
                type="text"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder={s.mcpUrlPlaceholder}
                className="w-full rounded-xl bg-black/40 border border-white/12 focus:border-violet-400/50 outline-none px-2.5 py-1 text-[10.5px] font-mono text-white placeholder:text-white/30"
              />
              <textarea
                rows={2}
                value={headersStr}
                onChange={(e) => setHeadersStr(e.target.value)}
                placeholder={s.mcpHeadersPlaceholder}
                className="w-full rounded-xl bg-black/40 border border-white/12 focus:border-violet-400/50 outline-none px-2.5 py-1 text-[10px] font-mono text-white placeholder:text-white/30 resize-none"
              />
            </>
          )}

          <div className="flex justify-end gap-1.5">
            <button
              type="button"
              onClick={() => setMode("none")}
              className="px-2.5 py-1 rounded-full bg-white/[0.06] hover:bg-white/[0.12] text-[10px] text-white/70 transition"
            >
              {s.cancelBtn}
            </button>
            <button
              type="submit"
              disabled={
                busy ||
                !name.trim() ||
                (transport === "stdio" ? !command.trim() : !url.trim())
              }
              className="px-3 py-1 rounded-full bg-violet-400 hover:bg-violet-300 disabled:opacity-40 text-slate-950 text-[10px] font-semibold inline-flex items-center gap-1 transition"
            >
              {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Plug className="w-3 h-3" />}
              <span>{s.mcpSaveBtn}</span>
            </button>
          </div>
        </form>
      )}

      {/* Paste JSON Importer */}
      {mode === "json" && (
        <div className="rounded-2xl bg-white/[0.045] border border-violet-400/25 p-2.5 space-y-2 shrink-0">
          <textarea
            rows={4}
            value={jsonDraft}
            onChange={(e) => setJsonDraft(e.target.value)}
            placeholder={s.mcpJsonPlaceholder}
            className="w-full rounded-xl bg-black/45 border border-white/12 focus:border-violet-400/50 outline-none p-2 text-[10px] font-mono text-white placeholder:text-white/30 resize-none leading-relaxed"
          />
          <div className="flex justify-end gap-1.5">
            <button
              type="button"
              onClick={() => setMode("none")}
              className="px-2.5 py-1 rounded-full bg-white/[0.06] hover:bg-white/[0.12] text-[10px] text-white/70 transition"
            >
              {s.cancelBtn}
            </button>
            <button
              type="button"
              disabled={busy || !jsonDraft.trim()}
              onClick={() => void handleImportJson()}
              className="px-3 py-1 rounded-full bg-violet-400 hover:bg-violet-300 disabled:opacity-40 text-slate-950 text-[10px] font-semibold inline-flex items-center gap-1 transition"
            >
              {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Code2 className="w-3 h-3" />}
              <span>{s.mcpImportBtn}</span>
            </button>
          </div>
        </div>
      )}

      {/* Server List */}
      <div className="flex-1 overflow-y-auto space-y-1.5 pr-1">
        {servers.length === 0 ? (
          <div className="rounded-2xl bg-white/[0.03] border border-white/10 p-5 text-center text-[11px] text-white/45 leading-relaxed">
            {s.mcpEmpty}
          </div>
        ) : (
          servers.map((st) => {
            const cfg = st.config;
            const isOpen = openId === cfg.id;
            return (
              <div
                key={cfg.id}
                className={`group rounded-2xl border transition-all duration-200 ${
                  isOpen
                    ? "bg-gradient-to-b from-white/[0.08] to-white/[0.04] border-violet-400/30 shadow-[0_8px_24px_rgba(0,0,0,0.28)]"
                    : "bg-white/[0.035] hover:bg-white/[0.06] border-white/[0.09] hover:border-white/[0.15]"
                } ${!cfg.enabled && !isOpen ? "opacity-60 hover:opacity-90" : ""}`}
              >
                <div
                  onClick={() => setOpenId(isOpen ? null : cfg.id)}
                  className="px-3 py-2.5 flex items-center gap-3 cursor-pointer select-none"
                >
                  <div
                    className={`w-9 h-9 rounded-xl border flex items-center justify-center shrink-0 relative ${
                      st.connected
                        ? "bg-emerald-500/12 border-emerald-400/25 text-emerald-300"
                        : st.last_error
                        ? "bg-rose-500/12 border-rose-400/25 text-rose-300"
                        : "bg-violet-500/14 border-violet-400/25 text-violet-300"
                    }`}
                  >
                    {cfg.transport === "http" ? (
                      <Globe className="w-4 h-4" />
                    ) : (
                      <Terminal className="w-4 h-4" />
                    )}
                    <span
                      className={`w-2 h-2 rounded-full absolute -top-0.5 -right-0.5 border border-[#0b0d14] ${
                        st.connecting
                          ? "bg-amber-400 animate-pulse"
                          : st.connected
                          ? "bg-emerald-400"
                          : st.last_error
                          ? "bg-rose-400"
                          : "bg-white/35"
                      }`}
                    />
                  </div>

                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5 min-w-0">
                      <span className="text-[12.5px] font-semibold text-white truncate">
                        {cfg.name}
                      </span>
                      <span className="px-1.5 py-[1px] rounded-md bg-white/[0.06] border border-white/10 text-[9px] font-mono uppercase text-white/55 shrink-0">
                        {cfg.transport}
                      </span>
                      {st.connected && (
                        <span className="px-1.5 py-[1px] rounded-md bg-emerald-500/15 border border-emerald-400/20 text-[9px] font-mono text-emerald-300 shrink-0">
                          {st.tool_count} {s.mcpToolsUnit}
                        </span>
                      )}
                    </div>
                    <div className="text-[10px] font-mono text-white/45 truncate mt-0.5">
                      {st.last_error
                        ? st.last_error
                        : cfg.transport === "http"
                        ? cfg.url
                        : `${cfg.command} ${cfg.args.join(" ")}`.trim()}
                    </div>
                  </div>

                  <div
                    className="flex items-center gap-1 shrink-0"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      title={s.mcpTrustTip}
                      onClick={() => {
                        void invoke<McpServerStatus[]>("set_mcp_server_trusted", {
                          id: cfg.id,
                          trusted: !cfg.trusted,
                        })
                          .then(onRefreshServers)
                          .catch((e) => onError(String(e)));
                      }}
                      className={`p-1.5 rounded-lg transition ${
                        cfg.trusted
                          ? "text-emerald-300 bg-emerald-500/15"
                          : "text-white/35 hover:text-white/80 hover:bg-white/10"
                      }`}
                    >
                      <ShieldCheck className="w-3.5 h-3.5" />
                    </button>

                    <button
                      type="button"
                      disabled={busy || !cfg.enabled}
                      onClick={() => {
                        onError(null);
                        setBusy(true);
                        void invoke<McpServerStatus[]>("reconnect_mcp_server", { id: cfg.id })
                          .then(onRefreshServers)
                          .catch((e) => onError(String(e)))
                          .finally(() => setBusy(false));
                      }}
                      title={s.mcpReconnectAll}
                      className="p-1.5 rounded-lg text-white/40 hover:text-sky-200 hover:bg-white/10 disabled:opacity-35 transition"
                    >
                      <RefreshCw className="w-3 h-3" />
                    </button>

                    <button
                      type="button"
                      onClick={() => {
                        void invoke<McpServerStatus[]>("delete_mcp_server", { id: cfg.id })
                          .then(onRefreshServers)
                          .catch((e) => onError(String(e)));
                      }}
                      title={s.uninstallBtn}
                      className="p-1.5 rounded-lg text-white/35 hover:text-rose-300 hover:bg-rose-500/15 transition"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>

                    <button
                      type="button"
                      role="switch"
                      aria-checked={cfg.enabled}
                      onClick={() => {
                        onError(null);
                        setBusy(true);
                        void invoke<McpServerStatus[]>("toggle_mcp_server", {
                          id: cfg.id,
                          enabled: !cfg.enabled,
                        })
                          .then(onRefreshServers)
                          .catch((e) => onError(String(e)))
                          .finally(() => setBusy(false));
                      }}
                      className={`relative w-8 h-[18px] rounded-full transition-colors ${
                        cfg.enabled
                          ? "bg-violet-400 shadow-[0_0_10px_rgba(167,139,250,0.35)]"
                          : "bg-white/20"
                      }`}
                    >
                      <span
                        className={`block w-3.5 h-3.5 rounded-full bg-white shadow transition-transform ${
                          cfg.enabled ? "translate-x-[16px]" : "translate-x-[2px]"
                        }`}
                      />
                    </button>

                    <button
                      type="button"
                      onClick={() => setOpenId(isOpen ? null : cfg.id)}
                      className="p-1 rounded-lg text-white/40 hover:text-white/80 transition"
                    >
                      <ChevronDown
                        className={`w-3.5 h-3.5 transition-transform duration-200 ${
                          isOpen ? "rotate-180 text-violet-300" : ""
                        }`}
                      />
                    </button>
                  </div>
                </div>

                {isOpen && (
                  <div className="px-2.5 pb-2.5">
                    <div className="rounded-xl bg-black/30 border border-white/[0.08] p-2.5 space-y-2">
                      {st.last_error && (
                        <div className="rounded-lg bg-rose-500/15 border border-rose-400/25 px-2.5 py-1.5 text-[10px] font-mono text-rose-200 break-words">
                          {st.last_error}
                        </div>
                      )}
                      {st.tools.length === 0 ? (
                        <div className="text-[10.5px] text-white/45 text-center py-2">
                          {st.connected ? "0 tools exposed" : s.mcpDisconnected}
                        </div>
                      ) : (
                        <div className="max-h-40 overflow-y-auto space-y-1 pr-0.5">
                          {st.tools.map((t) => (
                            <div
                              key={t.qualified_name}
                              className="rounded-lg bg-white/[0.03] border border-white/[0.06] px-2 py-1.5 flex items-start gap-2"
                            >
                              <Wrench className="w-3 h-3 text-violet-300/80 shrink-0 mt-0.5" />
                              <div className="min-w-0 flex-1">
                                <div className="text-[10.5px] font-mono font-medium text-white/90 truncate">
                                  {t.tool_name}
                                </div>
                                {t.description && (
                                  <div className="text-[9.5px] text-white/50 line-clamp-2 mt-0.5 leading-snug">
                                    {t.description}
                                  </div>
                                )}
                              </div>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
