/**
 * "Trust you can see": the history diff + restore plan, the per-device
 * activity labels, the plain Markdown export and the per-block saved
 * mark. Pure helpers plus small presentational components; every Tauri
 * call goes through `@outl/shared/api/commands`.
 */

export { wordDiff, type DiffOp, type DiffSegment } from "./diff";
export { restorePlan, type RestorePlan } from "./restore";
export { relativeAgo, deviceLabel } from "./activity";
export { createSavedMarks, type SavedMarks } from "./saved";
export { DiffText } from "./DiffText";
export { DeviceActivityList } from "./DeviceActivityList";
export { ExportCard } from "./ExportCard";
export { MergedEditsList, MergedEditsPanel } from "./MergedEditsList";
export { visibleMerges, withoutMerge } from "./merges";
export { createTrashRestore, deletedAgo, TRASH_EMPTY_TEXT, type TrashRestore } from "./trash";
