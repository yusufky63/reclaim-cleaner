import { AppWindow, ChevronRight, ClipboardCopy, Copy, FileStack, HardDrive, Map as MapIcon, Power, ScanSearch, ShieldCheck, Sparkles } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmClean, ProgressPanel, ReportDialog, UsageBar } from "../components/common";
import { useBytes, useLocale } from "../lib/hooks";
import { Badge, Button, Card, CardHeader, Checkbox, EmptyState, RiskDot, SectionTitle, Stat } from "../components/ui";
import { cx } from "../lib/cx";
import { api } from "../lib/api";
import { formatAgo, formatBytes, splitBytes } from "../lib/format";
import { buildRecommendations, cleanRecommendations, defaultSelection, scriptItems, type Rec } from "../lib/recommend";
import { buildScript } from "../lib/script";
import type { CleanReport, NodeView } from "../lib/types";
import { useApp, errorText, type PageId } from "../store/app";

function pageFor(rec: Rec): PageId {
  if (rec.kind === "artifacts") return "projects";
  if (rec.kind === "installers") return "large";
  return "junk";
}

function RecRow({ rec, checked, onToggle, onOpen }: { rec: Rec; checked: boolean; onToggle: (v: boolean) => void; onOpen: () => void }) {
  const { t } = useTranslation();
  const fmt = useBytes();
  const blocked = rec.blockedBy.length > 0;
  return (
    <li className="rail group flex items-center gap-3 px-4 py-2.5 hover:bg-surface">
      <Checkbox checked={checked} onChange={onToggle} disabled={blocked} label={rec.title} />
      <button type="button" onClick={onOpen} className="flex min-w-0 flex-1 items-center gap-3 text-left">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-[14px] font-medium text-ink">{rec.title}</span>
            {rec.admin && <Badge tone="primary">{t("common.needsAdmin")}</Badge>}
            {blocked && <Badge tone="warning">{t("junk.closeApp", { apps: rec.blockedBy.join(", ") })}</Badge>}
          </div>
          <div className="selectable truncate font-mono text-[12px] text-ink-muted" title={rec.paths[0]}>
            {rec.subtitle}
          </div>
        </div>
        <span className="num shrink-0 font-mono text-[14px] text-ink">{fmt(rec.bytes)}</span>
        <ChevronRight size={16} strokeWidth={1.75} className="shrink-0 text-ink-muted transition-transform transition-fast group-hover:translate-x-0.5" aria-hidden />
      </button>
    </li>
  );
}

function WhereSpace() {
  const { t } = useTranslation();
  const fmt = useBytes();
  const explorer = useApp((s) => s.explorer);
  const version = useApp((s) => s.explorerVersion);
  const running = useApp((s) => s.running.explorer);
  const info = useApp((s) => s.info);
  const scanExplorer = useApp((s) => s.scanExplorer);
  const go = useApp((s) => s.go);
  const [root, setRoot] = useState<NodeView | null>(null);
  useEffect(() => {
    if (!explorer) return;
    void api.explorerNode(0, 1, 7).then(setRoot).catch(() => setRoot(null));
  }, [explorer, version]);
  const items = root?.children.filter((c) => c.kind !== "small").slice(0, 6) ?? [];
  const max = items[0]?.bytes ?? 1;
  return (
    <Card>
      <CardHeader
        title={t("overview.whereSpace")}
        hint={explorer ? explorer.root : undefined}
        actions={
          explorer && (
            <Button size="sm" variant="ghost" onClick={() => go("explorer")}>
              {t("common.open")}
            </Button>
          )
        }
      />
      {running ? (
        <div className="p-4">
          <ProgressPanel kind="explorer" className="border-0 p-0" />
        </div>
      ) : !explorer ? (
        <div className="p-4">
          <p className="text-[13px] text-ink-secondary">{t("overview.whereSpaceEmpty")}</p>
          <Button className="mt-3" size="sm" icon={<MapIcon size={15} strokeWidth={1.75} />} onClick={() => info && void scanExplorer(info.userProfile)}>
            {t("overview.mapHome")}
          </Button>
        </div>
      ) : (
        <ul className="space-y-2.5 p-4">
          {items.map((c) => (
            <li key={c.id} className={cx("cat-" + c.category)}>
              <div className="mb-1 flex items-center justify-between gap-2 text-[13px]">
                <span className="flex min-w-0 items-center gap-2">
                  <span className="cat-dot" />
                  <span className="truncate text-ink">{c.name}</span>
                </span>
                <span className="num font-mono text-[12px] text-ink-secondary">{fmt(c.bytes)}</span>
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-muted">
                <div className="h-full rounded-full" style={{ width: `${(c.bytes / max) * 100}%`, background: "var(--cat)" }} />
              </div>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

function Drives() {
  const { t } = useTranslation();
  const drives = useApp((s) => s.drives);
  const fmt = useBytes();
  return (
    <Card>
      <CardHeader title={t("overview.drives")} />
      <ul className="divide-y divide-line">
        {drives.map((d) => (
          <li key={d.root} className="px-4 py-3">
            <div className="mb-1.5 flex items-center justify-between gap-2">
              <span className="flex min-w-0 items-center gap-2 text-[14px] text-ink">
                <HardDrive size={15} strokeWidth={1.75} className="text-ink-muted" aria-hidden />
                <span className="font-mono">{d.root.replace("\\", "")}</span>
                <span className="truncate text-ink-secondary">{d.label}</span>
              </span>
              <span className="num font-mono text-[12px] text-ink-secondary">{fmt(d.free)}</span>
            </div>
            <UsageBar segments={[{ value: d.total - d.free, className: d.free / d.total < 0.1 ? "bg-danger" : "bg-primary" }, { value: d.free, className: "swatch-free" }]} />
            <div className="mt-1 font-mono text-[11px] text-ink-muted">{t("overview.driveUsed", { used: fmt(d.total - d.free), total: fmt(d.total) })}</div>
          </li>
        ))}
      </ul>
    </Card>
  );
}

function QuickActions() {
  const { t } = useTranslation();
  const go = useApp((s) => s.go);
  const items: { page: PageId; icon: typeof FileStack; label: string }[] = [
    { page: "large", icon: FileStack, label: t("overview.quickLarge") },
    { page: "duplicates", icon: Copy, label: t("overview.quickDup") },
    { page: "apps", icon: AppWindow, label: t("overview.quickApps") },
    { page: "startup", icon: Power, label: t("overview.quickStartup") },
  ];
  return (
    <Card>
      <CardHeader title={t("overview.quick")} />
      <ul className="divide-y divide-line">
        {items.map(({ page, icon: Icon, label }) => (
          <li key={page}>
            <button type="button" onClick={() => go(page)} className="rail flex w-full items-center gap-3 px-4 py-2.5 text-left text-[14px] text-ink hover:bg-surface">
              <Icon size={16} strokeWidth={1.75} className="text-ink-muted" aria-hidden />
              <span className="flex-1">{label}</span>
              <ChevronRight size={15} strokeWidth={1.75} className="text-ink-muted" aria-hidden />
            </button>
          </li>
        ))}
      </ul>
    </Card>
  );
}

export function OverviewPage() {
  const { t, i18n } = useTranslation();
  const locale = useLocale();
  const fmt = useBytes();
  const { info, settings, rules, junk, junkAt, projects, installers, drives, running, scanAll, toast, go, refreshDrives } = useApp();
  const scanning = !!(running.junk || running.projects);

  const recs = useMemo(() => {
    if (!settings || !rules) return [];
    return buildRecommendations({ rules: rules.targets, junk, projects, installers, settings, lang: i18n.language, profile: info?.userProfile ?? "", t });
  }, [settings, rules, junk, projects, installers, i18n.language, info, t]);

  const [manual, setManual] = useState<Set<string> | null>(null);
  const selected = useMemo(() => manual ?? defaultSelection(recs), [manual, recs]);

  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<CleanReport | null>(null);

  const sys = drives.find((d) => d.system) ?? drives[0];
  const chosen = recs.filter((r) => selected.has(r.key));
  const chosenBytes = chosen.reduce((s, r) => s + r.bytes, 0);
  const safeBytes = recs.filter((r) => r.risk === "safe").reduce((s, r) => s + r.bytes, 0);
  const reviewBytes = recs.filter((r) => r.risk !== "safe").reduce((s, r) => s + r.bytes, 0);
  const safeCount = recs.filter((r) => r.risk === "safe").length;
  const reviewCount = recs.length - safeCount;
  const hasScan = junk !== null;

  const toggle = (key: string, on: boolean) => {
    const next = new Set(selected);
    if (on) next.add(key);
    else next.delete(key);
    setManual(next);
  };

  const copyScript = async () => {
    if (!rules || !settings || !info) return;
    const text = buildScript(scriptItems(chosen, rules.targets), { recycle: settings.cacheDelete === "recycle", generatedAt: new Date(), version: info.version });
    try {
      await navigator.clipboard.writeText(text);
      toast("positive", t("common.scriptCopied"));
    } catch (e) {
      toast("danger", errorText(e));
    }
  };

  const runClean = async () => {
    if (!settings) return;
    setBusy(true);
    try {
      const { report, elevationCancelled } = await cleanRecommendations(chosen, settings.cacheDelete, settings.filesDelete);
      setConfirm(false);
      setReport(report);
      if (elevationCancelled) toast("warning", t("overview.elevationCancelled"));
      setManual(null);
      await Promise.all([scanAll(), refreshDrives()]);
    } catch (e) {
      toast("danger", errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const freeNow = sys ? splitBytes(sys.free, locale) : null;
  const freeAfter = sys ? splitBytes(sys.free + chosenBytes, locale) : null;
  const safeSplit = splitBytes(safeBytes, locale);
  const reviewSplit = splitBytes(reviewBytes, locale);

  return (
    <div className="reveal space-y-5">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="display text-[24px] leading-tight text-ink">{t("overview.title")}</h1>
          <p className="mt-1 text-[14px] text-ink-secondary">{t("overview.subtitle")}</p>
        </div>
        {hasScan && (
          <Button variant="secondary" icon={<ScanSearch size={16} strokeWidth={1.75} />} loading={scanning} onClick={() => void scanAll()}>
            {scanning ? t("common.scanning") : t("common.rescan")}
          </Button>
        )}
      </div>

      <div className="module-grid ticks grid-cols-2 lg:grid-cols-4">
        <Stat
          label={t("overview.freeAfter")}
          value={freeAfter?.value ?? "—"}
          unit={freeAfter?.unit}
          sub={chosenBytes > 0 ? <span className="text-positive-fg">{t("overview.selectedGain", { size: fmt(chosenBytes) })}</span> : sys ? t("overview.of", { total: fmt(sys.total) }) : null}
        />
        <Stat label={t("overview.freeNow")} value={freeNow?.value ?? "—"} unit={freeNow?.unit} sub={sys ? `${sys.root.replace("\\", "")} · ${t("overview.of", { total: fmt(sys.total) })}` : null} />
        <Stat label={t("overview.safe")} dot={<RiskDot risk="safe" />} value={hasScan ? safeSplit.value : "—"} unit={hasScan ? safeSplit.unit : undefined} sub={hasScan ? t("common.items", { count: safeCount }) : null} />
        <Stat label={t("overview.review")} dot={<RiskDot risk="review" />} value={hasScan ? reviewSplit.value : "—"} unit={hasScan ? reviewSplit.unit : undefined} sub={hasScan ? t("common.items", { count: reviewCount }) : null} />
      </div>

      <div className="grid grid-cols-1 gap-5 lg:grid-cols-[minmax(0,1fr)_340px]">
        <Card className="flex min-h-[420px] flex-col">
          <CardHeader
            title={t("overview.recommendations")}
            hint={hasScan && !scanning ? t("overview.recommendationsHint") : undefined}
            actions={hasScan && junkAt && !scanning ? <span className="font-mono text-[11px] text-ink-muted">{t("common.lastScan", { when: formatAgo(junkAt, locale) })}</span> : undefined}
          />
          {scanning ? (
            <div className="space-y-3 p-4">
              {running.junk && <ProgressPanel kind="junk" label={t("junk.title")} />}
              {running.projects && <ProgressPanel kind="projects" label={t("projects.title")} />}
            </div>
          ) : !hasScan ? (
            <EmptyState
              className="flex-1"
              icon={<Sparkles size={20} strokeWidth={1.75} />}
              title={t("overview.emptyTitle")}
              body={t("overview.emptyBody")}
              action={
                <div className="flex flex-col items-center gap-4">
                  <Button variant="primary" size="lg" icon={<ScanSearch size={18} strokeWidth={1.75} />} onClick={() => void scanAll()}>
                    {t("overview.scanNow")}
                  </Button>
                  <ul className="flex flex-wrap justify-center gap-x-5 gap-y-1 text-[12px] text-ink-muted">
                    {[t("overview.promise1"), t("overview.promise2"), t("overview.promise3")].map((p) => (
                      <li key={p} className="flex items-center gap-1.5">
                        <ShieldCheck size={13} strokeWidth={1.75} className="text-positive-fg" aria-hidden />
                        {p}
                      </li>
                    ))}
                  </ul>
                </div>
              }
            />
          ) : recs.length === 0 ? (
            <EmptyState className="flex-1" icon={<ShieldCheck size={20} strokeWidth={1.75} />} title={t("overview.nothingFound")} />
          ) : (
            <>
              <div className="flex-1 overflow-y-auto">
                {(["safe", "review"] as const).map((bucket) => {
                  const list = recs.filter((r) => (bucket === "safe" ? r.risk === "safe" : r.risk !== "safe"));
                  if (list.length === 0) return null;
                  return (
                    <div key={bucket}>
                      <SectionTitle aside={fmt(list.reduce((s, r) => s + r.bytes, 0))}>
                        <span className="inline-flex items-center gap-2">
                          <RiskDot risk={bucket} />
                          {t(`risk.${bucket}`)}
                        </span>
                      </SectionTitle>
                      <ul className="divide-y divide-line">
                        {list.map((r) => (
                          <RecRow key={r.key} rec={r} checked={selected.has(r.key)} onToggle={(v) => toggle(r.key, v)} onOpen={() => go(pageFor(r))} />
                        ))}
                      </ul>
                    </div>
                  );
                })}
              </div>
              <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-line px-4 py-3">
                <span className="num font-mono text-[13px] text-ink">{chosen.length > 0 ? t("common.selectedSummary", { size: fmt(chosenBytes), count: chosen.length }) : t("common.nothingSelected")}</span>
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setManual(null)}
                  >
                    {t("common.selectSafe")}
                  </Button>
                  <Button size="sm" variant="secondary" icon={<ClipboardCopy size={15} strokeWidth={1.75} />} disabled={chosen.length === 0} onClick={() => void copyScript()}>
                    {t("common.copyScript")}
                  </Button>
                  <Button size="sm" variant="primary" disabled={chosen.length === 0} onClick={() => setConfirm(true)}>
                    {t("overview.cleanSelected")}
                  </Button>
                </div>
              </footer>
            </>
          )}
        </Card>
        <div className="space-y-5">
          <Drives />
          <WhereSpace />
          <QuickActions />
        </div>
      </div>

      <ConfirmClean
        open={confirm}
        onClose={() => setConfirm(false)}
        onConfirm={() => void runClean()}
        count={chosen.length}
        bytes={chosenBytes}
        method={settings?.cacheDelete ?? "permanent"}
        adminCount={chosen.filter((r) => r.admin && !info?.elevated).length}
        hasReview={chosen.some((r) => r.risk !== "safe")}
        busy={busy}
      >
        {chosen.some((r) => r.kind === "installers") && settings && (
          <p className="mt-3 text-[13px] text-ink-muted">
            {t("overview.installers")}: {t(`method.${settings.filesDelete}`)}
          </p>
        )}
      </ConfirmClean>
      <ReportDialog
        report={report}
        onClose={() => {
          if (report && report.freed > 0) toast("positive", t("overview.cleanedToast", { size: formatBytes(report.freed, locale) }));
          setReport(null);
        }}
      />
    </div>
  );
}
