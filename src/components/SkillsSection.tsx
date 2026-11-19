import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronDown,
  Download,
  FolderOpen,
  Info,
  Loader2,
  Search,
  Sparkles,
  Trash2,
} from "lucide-react";
import { getStrings } from "../i18n";
import type { GithubSkillCandidate, SkillDetail, SkillInfo, UiLang } from "../types";

export interface SkillsSectionProps {
  lang: UiLang;
  skills: SkillInfo[];
  onRefreshSkills: (next: SkillInfo[]) => void;
  onError: (msg: string | null) => void;
}

export function SkillsSection({
  lang,
  skills,
  onRefreshSkills,
  onError,
}: SkillsSectionProps) {
  const s = getStrings(lang).plugins;
  const [skillInput, setSkillInput] = useState("");
  const [skillBusy, setSkillBusy] = useState(false);
  const [ghResults, setGhResults] = useState<GithubSkillCandidate[]>([]);
  const [openSkillId, setOpenSkillId] = useState<string | null>(null);
  const [skillDetail, setSkillDetail] = useState<SkillDetail | null>(null);

  const runSmartSubmit = () => {
    const v = skillInput.trim();
    if (!v || skillBusy) return;
    const looksUrlOrRepo =
      /^https?:\/\/github\.com\//i.test(v) || /^[a-z0-9_.-]+\/[a-z0-9_.-]+$/i.test(v);
    const looksLocal = /^[a-z]:\\/i.test(v) || v.endsWith(".zip");
    onError(null);
    setSkillBusy(true);
    const req = looksLocal
      ? invoke<SkillInfo[]>("import_local_skill", { path: v })
      : looksUrlOrRepo
      ? invoke<SkillInfo[]>("install_github_skill_cmd", { spec: v })
      : invoke<GithubSkillCandidate[]>("search_github_skills_cmd", { query: v }).then((cands) => {
          setGhResults(cands);
          return skills;
        });
    void req
      .then((next) => {
        if (looksLocal || looksUrlOrRepo) {
          onRefreshSkills(next);
          setSkillInput("");
        }
      })
      .catch((err) => onError(String(err)))
      .finally(() => setSkillBusy(false));
  };

  return (
    <div className="flex-1 flex flex-col min-h-0 space-y-2">
      {/* Slim Role Clarifier */}
      <div className="rounded-xl bg-amber-500/[0.06] border border-amber-400/18 px-2.5 py-1.5 flex items-start gap-2 shrink-0">
        <Info className="w-3.5 h-3.5 text-amber-300/80 shrink-0 mt-0.5" />
        <p className="text-[10px] text-amber-100/75 leading-relaxed">{s.skillsExplainBanner}</p>
      </div>

      {/* Unified Search / Install Bar */}
      <div className="rounded-2xl bg-white/[0.04] border border-white/10 p-2 space-y-1.5 shrink-0">
        <div className="flex items-center gap-1.5">
          <div className="relative flex-1 min-w-0">
            <Search className="w-3 h-3 text-white/35 absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
            <input
              type="text"
              value={skillInput}
              onChange={(e) => setSkillInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  runSmartSubmit();
                }
              }}
              placeholder={s.skillsHint}
              className="w-full rounded-xl bg-black/35 border border-white/10 focus:border-amber-400/45 outline-none pl-7 pr-2.5 py-1.5 text-[11px] font-mono text-white placeholder:text-white/30 transition"
            />
          </div>
          <button
            type="button"
            onClick={() => void invoke("open_skills_folder", { id: null })}
            title={s.openFolder}
            className="w-7 h-7 rounded-xl bg-white/[0.06] hover:bg-white/15 border border-white/10 text-white/55 hover:text-white flex items-center justify-center shrink-0 transition"
          >
            <FolderOpen className="w-3.5 h-3.5" />
          </button>
        </div>

        <div className="flex items-center justify-end gap-1.5">
          <button
            type="button"
            disabled={skillBusy || !skillInput.trim()}
            onClick={() => {
              onError(null);
              setSkillBusy(true);
              void invoke<GithubSkillCandidate[]>("search_github_skills_cmd", {
                query: skillInput.trim(),
              })
                .then(setGhResults)
                .catch((e) => onError(String(e)))
                .finally(() => setSkillBusy(false));
            }}
            className="px-2.5 py-1 rounded-full bg-white/[0.07] hover:bg-white/[0.15] border border-white/10 disabled:opacity-35 text-[10px] text-white/90 inline-flex items-center gap-1 transition"
          >
            <Search className="w-2.5 h-2.5" />
            <span>{s.skillsSearchBtn}</span>
          </button>
          <button
            type="button"
            disabled={skillBusy || !skillInput.trim()}
            onClick={() => {
              const v = skillInput.trim();
              const looksLocal = /^[a-z]:\\/i.test(v) || v.endsWith(".zip");
              onError(null);
              setSkillBusy(true);
              const req = looksLocal
                ? invoke<SkillInfo[]>("import_local_skill", { path: v })
                : invoke<SkillInfo[]>("install_github_skill_cmd", { spec: v });
              void req
                .then((next) => {
                  onRefreshSkills(next);
                  setSkillInput("");
                })
                .catch((e) => onError(String(e)))
                .finally(() => setSkillBusy(false));
            }}
            className="px-2.5 py-1 rounded-full bg-amber-400 hover:bg-amber-300 disabled:opacity-35 text-[10px] font-semibold text-slate-950 inline-flex items-center gap-1 transition"
          >
            {skillBusy ? (
              <Loader2 className="w-2.5 h-2.5 animate-spin" />
            ) : (
              <Download className="w-2.5 h-2.5" />
            )}
            <span>{s.skillsInstallBtn}</span>
          </button>
        </div>
      </div>

      {/* Scrollable Results & Installed Skills */}
      <div className="flex-1 overflow-y-auto space-y-1.5 pr-1">
        {ghResults.length > 0 && (
          <div className="space-y-1.5 pb-2 border-b border-white/10">
            {ghResults.map((cand, idx) => (
              <div
                key={idx}
                className="rounded-2xl bg-sky-500/[0.08] border border-sky-400/20 px-3 py-2 flex items-center justify-between gap-2"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 text-[11.5px] font-medium text-sky-200 truncate">
                    <span className="truncate">{cand.title}</span>
                    {cand.stars > 0 && (
                      <span className="text-[9.5px] font-mono text-amber-300/90 shrink-0">
                        ★{cand.stars}
                      </span>
                    )}
                  </div>
                  <div className="text-[10px] text-white/55 truncate mt-0.5">
                    {cand.description}
                  </div>
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  {cand.is_repo && (
                    <button
                      type="button"
                      disabled={skillBusy}
                      onClick={() => {
                        onError(null);
                        setSkillBusy(true);
                        void invoke<GithubSkillCandidate[]>("search_github_skills_cmd", {
                          query: cand.install_spec,
                        })
                          .then(setGhResults)
                          .catch((e) => onError(String(e)))
                          .finally(() => setSkillBusy(false));
                      }}
                      className="px-2 py-1 rounded-full bg-white/10 hover:bg-white/20 text-[10px] text-white/85 transition"
                    >
                      Browse
                    </button>
                  )}
                  <button
                    type="button"
                    disabled={skillBusy}
                    onClick={() => {
                      onError(null);
                      setSkillBusy(true);
                      void invoke<SkillInfo[]>("install_github_skill_cmd", {
                        spec: cand.install_spec,
                      })
                        .then((next) => {
                          onRefreshSkills(next);
                          setGhResults([]);
                        })
                        .catch((e) => onError(String(e)))
                        .finally(() => setSkillBusy(false));
                    }}
                    className="px-2.5 py-1 rounded-full bg-sky-500 hover:bg-sky-400 text-[10px] font-semibold text-slate-950 transition"
                  >
                    {s.skillsInstallBtn}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        {skills.length === 0 ? (
          <div className="rounded-2xl bg-white/[0.03] border border-white/10 p-5 text-center text-[11px] text-white/45">
            {s.skillsEmpty}
          </div>
        ) : (
          skills.map((sk) => {
            const isOpen = openSkillId === sk.id;
            const toggleOpen = () => {
              if (isOpen) {
                setOpenSkillId(null);
                setSkillDetail(null);
              } else {
                setOpenSkillId(sk.id);
                void invoke<SkillDetail>("get_skill_detail", { id: sk.id })
                  .then(setSkillDetail)
                  .catch(() => {});
              }
            };

            return (
              <div
                key={sk.id}
                className={`group rounded-2xl border transition-all duration-200 ${
                  isOpen
                    ? "bg-gradient-to-b from-white/[0.08] to-white/[0.04] border-amber-400/30 shadow-[0_8px_24px_rgba(0,0,0,0.28)]"
                    : "bg-white/[0.035] hover:bg-white/[0.06] border-white/[0.09] hover:border-white/[0.15]"
                } ${!sk.enabled && !isOpen ? "opacity-60 hover:opacity-90" : ""}`}
              >
                <div
                  onClick={toggleOpen}
                  className="px-3 py-2.5 flex items-center gap-3 cursor-pointer select-none"
                >
                  <div className="w-9 h-9 rounded-xl bg-amber-500/14 border border-amber-400/25 flex items-center justify-center shrink-0">
                    <Sparkles className="w-4 h-4 text-amber-300" />
                  </div>

                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5 min-w-0">
                      <span className="px-1.5 py-[1px] rounded-md bg-amber-500/15 border border-amber-400/25 text-[10.5px] font-mono font-semibold text-amber-200 shrink-0">
                        /{sk.id}
                      </span>
                      <span className="text-[12px] font-semibold text-white truncate">
                        {sk.name !== sk.id ? sk.name : s.skillsWorkflowBadge}
                      </span>
                      <span className="px-1.5 py-[1px] rounded-md bg-white/[0.06] text-[9px] font-mono text-white/45 shrink-0">
                        {sk.file_count}f
                      </span>
                    </div>
                    <p className="text-[10.5px] text-white/50 line-clamp-1 mt-0.5 leading-snug">
                      {sk.description}
                    </p>
                  </div>

                  <div
                    className="flex items-center gap-1 shrink-0"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      onClick={() => void invoke("open_skills_folder", { id: sk.id })}
                      title={s.openFolder}
                      className="p-1.5 rounded-lg opacity-65 group-hover:opacity-100 text-white/45 hover:text-white hover:bg-white/10 transition"
                    >
                      <FolderOpen className="w-3.5 h-3.5" />
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        void invoke<SkillInfo[]>("uninstall_skill", { id: sk.id })
                          .then(onRefreshSkills)
                          .catch((e) => onError(String(e)));
                      }}
                      title={s.uninstallBtn}
                      className="p-1.5 rounded-lg opacity-65 group-hover:opacity-100 text-white/45 hover:text-rose-300 hover:bg-rose-500/15 transition"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={sk.enabled}
                      onClick={() => {
                        void invoke<SkillInfo[]>("toggle_skill", {
                          id: sk.id,
                          enabled: !sk.enabled,
                        })
                          .then(onRefreshSkills)
                          .catch(() => {});
                      }}
                      className={`relative w-8 h-[18px] rounded-full transition-colors ${
                        sk.enabled
                          ? "bg-amber-400 shadow-[0_0_10px_rgba(251,191,36,0.3)]"
                          : "bg-white/20"
                      }`}
                    >
                      <span
                        className={`block w-3.5 h-3.5 rounded-full bg-white shadow transition-transform ${
                          sk.enabled ? "translate-x-[16px]" : "translate-x-[2px]"
                        }`}
                      />
                    </button>
                    <button
                      type="button"
                      onClick={toggleOpen}
                      aria-label={s.skillsInspect}
                      className="p-1 rounded-lg text-white/40 hover:text-white/80 transition"
                    >
                      <ChevronDown
                        className={`w-3.5 h-3.5 transition-transform duration-200 ${
                          isOpen ? "rotate-180 text-amber-300" : ""
                        }`}
                      />
                    </button>
                  </div>
                </div>

                {isOpen && skillDetail && skillDetail.info.id === sk.id && (
                  <div className="px-2.5 pb-2.5">
                    <div className="rounded-xl bg-black/30 border border-white/[0.08] p-2.5 space-y-1.5">
                      <div className="flex items-center flex-wrap gap-1 text-[9.5px] font-mono text-white/45">
                        <span className="px-1.5 py-0.5 rounded bg-white/[0.06]">{sk.source}</span>
                        {skillDetail.files.map((f) => (
                          <span key={f} className="px-1.5 py-0.5 rounded bg-white/[0.04]">
                            {f}
                          </span>
                        ))}
                      </div>
                      <pre className="rounded-lg bg-black/45 border border-white/[0.08] p-2 text-[10px] font-mono text-white/85 whitespace-pre-wrap max-h-40 overflow-y-auto leading-relaxed">
                        {skillDetail.skill_md}
                      </pre>
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
