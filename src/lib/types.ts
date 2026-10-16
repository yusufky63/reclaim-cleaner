// Mirrors of the Rust types serialized by the Tauri commands (camelCase on the wire).

export type Risk = "safe" | "review" | "caution";
export type Group = "system" | "browser" | "apps" | "dev" | "games" | "ai" | "media";
export type Mode = "contents" | "folder" | "files" | "report";
export type DeleteMethod = "permanent" | "recycle";
export type Category = "projects" | "media" | "caches" | "apps" | "ai" | "system" | "other";
export type NodeKind = "dir" | "file" | "small";

export interface OsInfo {
  name: string;
  displayVersion: string;
  build: string;
}

export interface LibraryFolders {
  desktop: string | null;
  documents: string | null;
  downloads: string | null;
  pictures: string | null;
  music: string | null;
  videos: string | null;
}

export interface AppInfo {
  version: string;
  elevated: boolean;
  portable: boolean;
  dataDir: string;
  os: OsInfo;
  libraries: LibraryFolders;
  suggestedRoots: string[];
  settingsError: string | null;
  ruleProblems: string[];
  userProfile: string;
}

export interface Drive {
  root: string;
  label: string;
  fileSystem: string;
  total: number;
  free: number;
  removable: boolean;
  system: boolean;
}

export interface SystemFile {
  name: string;
  path: string;
  bytes: number;
}

export interface RecycleBinInfo {
  bytes: number;
  items: number;
}

export interface Settings {
  language: "system" | "en" | "tr";
  theme: "system" | "light" | "dark";
  cacheDelete: DeleteMethod;
  filesDelete: DeleteMethod;
  showCaution: boolean;
  exclusions: string[];
  projectRoots: string[];
  idleDays: number;
  largeMinMb: number;
  duplicateMinKb: number;
  installerAgeDays: number;
  checkUpdates: boolean;
  onboarded: boolean;
}

export interface TargetRule {
  id: string;
  group: Group;
  name: string;
  nameTr: string;
  description: string;
  descriptionTr: string;
  paths: string[];
  mode: Mode;
  include: string[];
  minAgeHours: number | null;
  risk: Risk;
  admin: boolean;
  closeApps: string[];
  command: string | null;
  verified: boolean;
  source: string | null;
  user: boolean;
}

export interface ArtifactRule {
  id: string;
  dirName: string;
  markers: string[];
  contains: string[];
  risk: Risk;
  description: string;
  descriptionTr: string;
}

export interface RulesPayload {
  targets: TargetRule[];
  artifacts: ArtifactRule[];
}

export interface ProgressSnapshot {
  files: number;
  dirs: number;
  bytes: number;
  errors: number;
  done: number;
  total: number;
  stage: string;
  current: string;
}

export interface ProgressEvent extends ProgressSnapshot {
  kind: string;
}

export interface FoundPath {
  path: string;
  bytes: number;
  files: number;
}

export interface TargetResult {
  id: string;
  bytes: number;
  files: number;
  found: FoundPath[];
  running: string[];
  partial: boolean;
  commandAvailable: boolean;
}

export interface Failure {
  path: string;
  reason: string;
}

export interface CleanReport {
  freed: number;
  recycled: number;
  files: number;
  dirs: number;
  failed: number;
  failures: Failure[];
  cancelled: boolean;
  items: string[];
}

export interface CleanOutcome {
  report: CleanReport;
  elevationCancelled: boolean;
}

export interface CommandResult {
  code: number | null;
  output: string;
  cancelled: boolean;
}

export interface ScanSummary {
  root: string;
  bytes: number;
  files: number;
  dirs: number;
  denied: number;
  millis: number;
  scannedAt: number;
  cancelled: boolean;
}

export interface NodeView {
  id: number;
  name: string;
  path: string;
  kind: NodeKind;
  category: Category;
  bytes: number;
  files: number;
  mtime: number;
  denied: boolean;
  reclaimSafe: number;
  reclaimReview: number;
  marked: Risk | null;
  childCount: number;
  children: NodeView[];
  restBytes: number;
  restCount: number;
}

export interface Crumb {
  id: number;
  name: string;
}

export interface FileView {
  id: number;
  name: string;
  path: string;
  bytes: number;
  mtime: number;
  category: Category;
}

export interface ExtStat {
  ext: string;
  bytes: number;
  files: number;
}

export interface ArtifactHit {
  path: string;
  ruleId: string;
  dirName: string;
  risk: Risk;
  bytes: number;
  files: number;
}

export interface Project {
  path: string;
  name: string;
  lastActivity: number;
  hasGit: boolean;
  artifacts: ArtifactHit[];
  bytes: number;
}

export interface DupFile {
  path: string;
  mtime: number;
}

export interface DupGroup {
  hash: string;
  bytes: number;
  files: DupFile[];
  wasted: number;
}

export interface EmptyFolder {
  path: string;
  mtime: number;
  nested: number;
}

export interface InstallerFile {
  path: string;
  name: string;
  bytes: number;
  mtime: number;
  ext: string;
}

export interface InstalledApp {
  id: string;
  name: string;
  publisher: string;
  version: string;
  installed: string | null;
  bytes: number | null;
  location: string | null;
  icon: string | null;
  uninstall: string | null;
  quietUninstall: string | null;
  scope: string;
  source: "desktop" | "store";
}

export type LeftoverConfidence = "high" | "low";
export type LeftoverReason = "install" | "name" | "publisherName" | "publisher" | "similar";

export interface Leftover {
  path: string;
  bytes: number;
  files: number;
  confidence: LeftoverConfidence;
  reason: LeftoverReason;
  /** Below Program Files or ProgramData, where removing usually needs administrator rights. */
  admin: boolean;
}

export interface UninstallResult {
  cancelled: boolean;
  exitCode: number | null;
  removed: boolean;
  /** High confidence first. Empty unless the app was removed. */
  leftovers: Leftover[];
}

export type StartupSource = "runUser" | "runMachine" | "runMachine32" | "folderUser" | "folderCommon";

export interface StartupItem {
  name: string;
  command: string;
  source: StartupSource;
  enabled: boolean;
  needsAdmin: boolean;
}

export interface HistoryEntry {
  time: string;
  kind: string;
  method: DeleteMethod;
  freed: number;
  recycled: number;
  files: number;
  failed: number;
  items: string[];
  elevated: boolean;
}

export type ToolId = "dism-cleanup" | "delivery-optimization" | "hibernate-off" | "hibernate-on" | "windows-old" | "disk-cleanup" | "storage-sense" | "system-protection";
