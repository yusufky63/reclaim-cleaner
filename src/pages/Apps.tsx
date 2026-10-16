import { AppWindow, FolderOpen, Search, Trash2, Unlink } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useBytes, useLocale } from "../lib/hooks";
import { AppIcon } from "../components/FileIcons";
import { Badge, Banner, Button, Card, Dialog, EmptyState, PageHeader, Segmented, Select, Skeleton } from "../components/ui";
import { api } from "../lib/api";
import { formatBytes } from "../lib/format";
import type { InstalledApp, UninstallResult } from "../lib/types";
import { errorText, useApp } from "../store/app";

type Sort = "size" | "name" | "date";

export function AppsPage() {
  const { t } = useTranslation();
  const fmt = useBytes();
  const locale = useLocale();
  const { toast, refreshDrives } = useApp();
  const [tab, setTab] = useState<"desktop" | "store">("desktop");
  const [desktop, setDesktop] = useState<InstalledApp[] | null>(null);
  const [store, setStore] = useState<InstalledApp[] | null>(null);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<Sort>("size");
  const [target, setTarget] = useState<InstalledApp | null>(null);
  const [busy, setBusy] = useState(false);
  const [leftover, setLeftover] = useState<{ app: InstalledApp; result: UninstallResult } | null>(null);
  const [onlyBroken, setOnlyBroken] = useState(false);
  const [entryTarget, setEntryTarget] = useState<InstalledApp | null>(null);

  const loadDesktop = () => void api.apps().then(setDesktop).catch((e) => toast("danger", errorText(e)));
  const loadStore = () => void api.storeApps().then(setStore).catch((e) => toast("danger", errorText(e)));

  useEffect(() => {
    loadDesktop();
    // Load once on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    if (tab === "store" && store === null) loadStore();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab]);

  const source = tab === "desktop" ? desktop : store;
  const list = useMemo(() => {
    const q = query.trim().toLowerCase();
    const filtered = (source ?? []).filter((a) => (!onlyBroken || a.broken) && (!q || a.name.toLowerCase().includes(q) || a.publisher.toLowerCase().includes(q)));
    return [...filtered].sort((a, b) => {
      if (sort === "size") return (b.bytes ?? -1) - (a.bytes ?? -1);
      if (sort === "date") return (b.installed ?? "").localeCompare(a.installed ?? "");
      return a.name.localeCompare(b.name, locale);
    });
  }, [source, query, sort, locale, onlyBroken]);
  const brokenCount = (source ?? []).filter((a) => a.broken).length;
  const totalBytes = (source ?? []).reduce((s, a) => s + (a.bytes ?? 0), 0);

  const uninstall = async () => {
    if (!target) return;
    setBusy(true);
    const app = target;
    try {
      const r = await api.uninstall(app.id);
      setTarget(null);
      if (r.cancelled) {
        toast("warning", t("tools.declined"));
      } else if (r.removed) {
        toast("positive", t("apps.removed", { name: app.name }));
        if (r.leftover) setLeftover({ app, result: r });
      } else {
        toast("warning", t("apps.notRemoved", { name: app.name }));
      }
      if (app.source === "store") loadStore();
      else loadDesktop();
      void refreshDrives();
    } catch (e) {
      toast("danger", errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const removeEntry = async () => {
    if (!entryTarget) return;
    setBusy(true);
    try {
      const backup = await api.appRemoveEntry(entryTarget.id);
      setEntryTarget(null);
      toast("positive", t("apps.entryRemoved", { path: backup }));
      loadDesktop();
    } catch (e) {
      toast("danger", errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const removeLeftover = async () => {
    if (!leftover?.result.leftover) return;
    setBusy(true);
    try {
      await api.pathsClean("apps", [leftover.result.leftover.path], "recycle");
      setLeftover(null);
      void refreshDrives();
    } catch (e) {
      toast("danger", errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-5">
      <PageHeader
        title={t("apps.title")}
        subtitle={t("apps.subtitle")}
        meta={source ? <span className="num font-mono">{t("common.items", { count: source.length })} · {fmt(totalBytes)}</span> : null}
      />
      <Card>
        <div className="flex flex-wrap items-center gap-3 px-4 py-3">
          <Segmented
            value={tab}
            onChange={setTab}
            options={[
              { value: "desktop", label: t("apps.desktop") },
              { value: "store", label: t("apps.store") },
            ]}
          />
          <label className="field flex min-w-[220px] flex-1 items-center gap-2">
            <Search size={15} strokeWidth={1.75} className="text-ink-muted" aria-hidden />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("apps.search")} aria-label={t("apps.search")} className="h-full w-full bg-transparent text-[14px] text-ink outline-none placeholder:text-ink-muted" />
          </label>
          <Select
            label={t("common.size")}
            value={sort}
            onChange={setSort}
            options={[
              { value: "size", label: t("apps.sortSize") },
              { value: "name", label: t("apps.sortName") },
              { value: "date", label: t("apps.sortDate") },
            ]}
          />
        </div>
      </Card>
      {tab === "desktop" && brokenCount > 0 && (
        <Banner
          tone="warning"
          icon={<Unlink size={15} strokeWidth={1.75} />}
          action={
            <Button size="sm" variant="ghost" onClick={() => setOnlyBroken(!onlyBroken)}>
              {onlyBroken ? t("models.sources.all") : t("common.show")}
            </Button>
          }
        >
          {t("apps.brokenFilter", { count: brokenCount })}. {t("apps.brokenHint")}
        </Banner>
      )}
      <Card className="overflow-hidden">
        {source === null ? (
          <div className="space-y-2 p-4">
            {tab === "store" && <div className="text-[13px] text-ink-muted">{t("apps.loadingStore")}</div>}
            {Array.from({ length: 6 }, (_, i) => (
              <Skeleton key={i} className="h-11" />
            ))}
          </div>
        ) : list.length === 0 ? (
          <EmptyState icon={<AppWindow size={20} strokeWidth={1.75} />} title={t("apps.none")} />
        ) : (
          <ul className="divide-y divide-line">
            {list.map((a) => (
              <li key={a.id} className="rail flex items-center gap-3 px-4 py-2.5 hover:bg-surface">
                <AppIcon path={a.icon} name={a.name} size={32} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className={a.broken ? "truncate text-[14px] font-medium text-ink-secondary line-through decoration-ink-muted" : "truncate text-[14px] font-medium text-ink"}>{a.name}</span>
                    {a.version && <span className="shrink-0 font-mono text-[11px] text-ink-muted">{a.version}</span>}
                    {a.broken && (
                      <Badge tone="warning" title={t("apps.brokenHint")}>
                        {t("apps.broken")}
                      </Badge>
                    )}
                  </div>
                  <div className="flex items-center gap-2 text-[12px] text-ink-muted">
                    <span className="truncate">{a.publisher}</span>
                    {a.installed && <span className="shrink-0 font-mono">{a.installed}</span>}
                    <Badge>{a.scope === "machine" ? t("apps.scopeMachine") : t("apps.scopeUser")}</Badge>
                  </div>
                </div>
                <span className="num w-[84px] shrink-0 text-right font-mono text-[13px] text-ink">{a.bytes ? fmt(a.bytes) : <span className="text-ink-muted">{t("apps.unknownSize")}</span>}</span>
                <div className="flex shrink-0 items-center gap-1">
                  {a.location && (
                    <Button size="icon" variant="ghost" aria-label={t("common.open")} title={a.location} onClick={() => void api.open(a.location!).catch((e) => toast("danger", errorText(e)))}>
                      <FolderOpen size={15} strokeWidth={1.75} />
                    </Button>
                  )}
                  {a.broken ? (
                    <Button size="sm" variant="secondary" icon={<Unlink size={14} strokeWidth={1.75} />} onClick={() => setEntryTarget(a)}>
                      {t("apps.removeEntry")}
                    </Button>
                  ) : (
                    <Button size="sm" variant="secondary" icon={<Trash2 size={14} strokeWidth={1.75} />} onClick={() => setTarget(a)}>
                      {t("apps.uninstall")}
                    </Button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>
      <Dialog
        open={!!target}
        onClose={() => !busy && setTarget(null)}
        title={t("apps.confirmTitle", { name: target?.name ?? "" })}
        footer={
          <>
            <Button variant="ghost" onClick={() => setTarget(null)} disabled={busy}>
              {t("common.cancel")}
            </Button>
            <Button variant="primary" onClick={() => void uninstall()} loading={busy} data-autofocus>
              {t("apps.uninstall")}
            </Button>
          </>
        }
      >
        <p className="text-[14px] text-ink-secondary">{t("apps.confirmBody")}</p>
        {target?.uninstall && <code className="selectable mt-3 block truncate rounded bg-surface-muted px-2 py-1.5 font-mono text-[11px] text-ink">{target.uninstall}</code>}
      </Dialog>
      <Dialog
        open={!!leftover}
        onClose={() => setLeftover(null)}
        title={t("apps.leftoverTitle")}
        footer={
          <>
            <Button variant="ghost" onClick={() => setLeftover(null)}>
              {t("common.close")}
            </Button>
            <Button variant="primary" onClick={() => void removeLeftover()} loading={busy}>
              {t("method.recycle")}
            </Button>
          </>
        }
      >
        {leftover?.result.leftover && (
          <>
            <p className="text-[14px] text-ink-secondary">{t("apps.leftoverBody", { name: leftover.app.name, size: formatBytes(leftover.result.leftover.bytes, locale) })}</p>
            <code className="selectable mt-3 block truncate rounded bg-surface-muted px-2 py-1.5 font-mono text-[11px] text-ink">{leftover.result.leftover.path}</code>
          </>
        )}
      </Dialog>
      <Dialog
        open={!!entryTarget}
        onClose={() => !busy && setEntryTarget(null)}
        title={t("apps.removeEntryTitle", { name: entryTarget?.name ?? "" })}
        footer={
          <>
            <Button variant="ghost" onClick={() => setEntryTarget(null)} disabled={busy}>
              {t("common.cancel")}
            </Button>
            <Button variant="primary" onClick={() => void removeEntry()} loading={busy} data-autofocus>
              {t("apps.removeEntry")}
            </Button>
          </>
        }
      >
        <p className="text-[14px] text-ink-secondary">{t("apps.removeEntryBody")}</p>
        {entryTarget?.uninstall && <code className="selectable mt-3 block truncate rounded bg-surface-muted px-2 py-1.5 font-mono text-[11px] text-ink">{entryTarget.uninstall}</code>}
      </Dialog>
    </div>
  );
}
