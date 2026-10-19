/**
 * Crash-reporting consent: the two commands behind the Privacy toggle
 * (RFC 0377 section 7).
 *
 * Its own file rather than a block in `commands.ts`, which sits at its
 * size ratchet — the same reason `trust.ts` and `overview.ts` are
 * separate. Nothing here is a wire DTO (both commands carry a bare
 * `bool`), so there is nothing for `outl-tauri-shared/tests/wire_*` to
 * pin and this file is deliberately absent from `ts_parser::MIRROR_FILES`.
 *
 * Both GUI clients register `telemetry_commands!`, so the wrapper lives
 * here once instead of per client. That is not housekeeping: reporting
 * is on by default, so a client without this pair is a client with no
 * opt-out, and `outl_shortcuts::capability_support` records
 * `Capability::CrashReportingOptOut` as supported on both.
 *
 * The copy that goes beside the toggle is `CRASH_REPORTS_SETTING` in
 * `@outl/shared/onboarding` — one owner for the promise, on both
 * clients.
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * Whether this device currently sends crash reports.
 *
 * `true` on a config that never mentioned `[telemetry]`: the default is
 * on, which is why the toggle has to load its initial state from here
 * rather than assuming `false`.
 */
export function getTelemetryEnabled(): Promise<boolean> {
  return invoke<boolean>("get_telemetry_enabled");
}

/**
 * Write the consent flag and resolve to what is now on disk.
 *
 * Resolves the saved value rather than `void` so the caller moves its
 * switch only after the write lands — the shape `setReminderSettings`
 * uses. Flipping optimistically means a failed save leaves a switch
 * reading "off" over a reporter that is still on, which is the one lie
 * this surface cannot tell.
 */
export function setTelemetryEnabled(enabled: boolean): Promise<boolean> {
  return invoke<boolean>("set_telemetry_enabled", { enabled });
}
