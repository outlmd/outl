// ESLint 9+ flat config. `@raycast/eslint-config` v2 already exports a flat
// array (CJS), so this file just re-exports it — `.eslintrc.json` stopped
// being read the moment eslint crossed v9.
const raycast = require("@raycast/eslint-config");

module.exports = raycast;
