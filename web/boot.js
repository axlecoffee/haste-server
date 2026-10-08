// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
const script = document.querySelector('script[data-js][data-wasm]');
const { default: init } = await import(script.dataset.js);
await init({ module_or_path: script.dataset.wasm });