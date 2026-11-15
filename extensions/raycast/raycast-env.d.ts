/// <reference types="@raycast/api">

/* 🚧 🚧 🚧
 * This file is auto-generated from the extension's manifest.
 * Do not modify manually. Instead, update the `package.json` file.
 * 🚧 🚧 🚧 */

/* eslint-disable @typescript-eslint/ban-types */

type ExtensionPreferences = {
  /** Workspace Path - Absolute path to your outl workspace (the folder that contains .outl/, journals/, pages/). */
  "workspace": string,
  /** outl Binary - Path to the outl binary. Leave as 'outl' if it is on Raycast's PATH; otherwise use an absolute path (e.g. /opt/homebrew/bin/outl). Raycast often does not inherit your shell PATH. */
  "outlBin": string
}

/** Preferences accessible in all the extension's commands */
declare type Preferences = ExtensionPreferences

declare namespace Preferences {
  /** Preferences accessible in the `quick-capture` command */
  export type QuickCapture = ExtensionPreferences & {}
  /** Preferences accessible in the `search` command */
  export type Search = ExtensionPreferences & {}
  /** Preferences accessible in the `open-today` command */
  export type OpenToday = ExtensionPreferences & {}
  /** Preferences accessible in the `new-page` command */
  export type NewPage = ExtensionPreferences & {}
  /** Preferences accessible in the `merge-pages` command */
  export type MergePages = ExtensionPreferences & {}
}

declare namespace Arguments {
  /** Arguments passed to the `quick-capture` command */
  export type QuickCapture = {
  /** What's on your mind? */
  "text": string
}
  /** Arguments passed to the `search` command */
  export type Search = {}
  /** Arguments passed to the `open-today` command */
  export type OpenToday = {}
  /** Arguments passed to the `new-page` command */
  export type NewPage = {}
  /** Arguments passed to the `merge-pages` command */
  export type MergePages = {}
}

