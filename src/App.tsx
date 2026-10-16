import { AlertTriangle, MonitorX } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Shell } from "./components/Shell";
import { Banner, EmptyState } from "./components/ui";
import { isDesktop, onProgress } from "./lib/api";
import { checkForUpdate } from "./lib/updates";
import { AiModelsPage } from "./pages/AiModels";
import { AppsPage } from "./pages/Apps";
import { AssistantsPage } from "./pages/Assistants";
import { DownloadsPage } from "./pages/Downloads";
import { CleanerPage } from "./pages/Cleaner";
import { DuplicatesPage } from "./pages/Duplicates";
import { EmptyFoldersPage } from "./pages/EmptyFolders";
import { ExplorerPage } from "./pages/Explorer";
import { HistoryPage } from "./pages/History";
import { LargeFilesPage } from "./pages/LargeFiles";
import { OverviewPage } from "./pages/Overview";
import { ProjectsPage } from "./pages/Projects";
import { SettingsPage } from "./pages/Settings";
import { StartupPage } from "./pages/Startup";
import { ToolsPage } from "./pages/Tools";
import { useApp } from "./store/app";

function Page() {
  const page = useApp((s) => s.page);
  switch (page) {
    case "overview":
      return <OverviewPage />;
    case "explorer":
      return <ExplorerPage />;
    case "large":
      return <LargeFilesPage />;
    case "duplicates":
      return <DuplicatesPage />;
    case "empty":
      return <EmptyFoldersPage />;
    case "junk":
      return <CleanerPage />;
    case "projects":
      return <ProjectsPage />;
    case "models":
      return <AiModelsPage />;
    case "assistants":
      return <AssistantsPage />;
    case "downloads":
      return <DownloadsPage />;
    case "apps":
      return <AppsPage />;
    case "startup":
      return <StartupPage />;
    case "tools":
      return <ToolsPage />;
    case "history":
      return <HistoryPage />;
    case "settings":
      return <SettingsPage />;
  }
}

export function App() {
  const { t } = useTranslation();
  const boot = useApp((s) => s.boot);
  const ready = useApp((s) => s.ready);
  const bootError = useApp((s) => s.bootError);
  const settingsError = useApp((s) => s.info?.settingsError);
  const page = useApp((s) => s.page);
  const go = useApp((s) => s.go);

  useEffect(() => {
    void boot().then(() => {
      const { settings, info, toast } = useApp.getState();
      if (settings?.checkUpdates && info) {
        void checkForUpdate(info.version).then((r) => {
          if (r.kind === "available") toast("neutral", t("settings.updateAvailable", { version: r.version }));
        });
      }
    });
    const unlisten = onProgress((e) => useApp.getState().setProgress(e));
    return () => {
      void unlisten.then((f) => f());
    };
    // Boot once; `t` only formats the optional update toast.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [boot]);

  if (!isDesktop()) {
    return (
      <div className="flex h-full items-center justify-center">
        <EmptyState icon={<MonitorX size={20} strokeWidth={1.75} />} title={t("app.fullName")} body={t("common.desktopOnly")} />
      </div>
    );
  }
  if (!ready) return <div className="h-full bg-canvas" />;
  return (
    <Shell>
      {bootError && (
        <div className="mb-4">
          <Banner tone="danger" icon={<AlertTriangle size={16} strokeWidth={1.75} />}>
            {bootError}
          </Banner>
        </div>
      )}
      {settingsError && page !== "settings" && (
        <div className="mb-4">
          <Banner
            tone="danger"
            icon={<AlertTriangle size={16} strokeWidth={1.75} />}
            action={
              <button type="button" className="font-medium underline" onClick={() => go("settings")}>
                {t("nav.settings")}
              </button>
            }
          >
            {t("settings.settingsError")}
          </Banner>
        </div>
      )}
      <Page />
    </Shell>
  );
}
