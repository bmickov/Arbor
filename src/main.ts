import "@fontsource/atkinson-hyperlegible-next/400.css";
import "@fontsource/atkinson-hyperlegible-next/500.css";
import "@fontsource/atkinson-hyperlegible-next/600.css";
import "@fontsource/atkinson-hyperlegible-next/700.css";
import "@fontsource/commit-mono/400.css";
import "@fontsource/commit-mono/600.css";
import "./styles.css";
import { mount } from "svelte";
import App from "./App.svelte";

import { toasts } from "./lib/ui.svelte";

// Surface unexpected errors instead of failing silently.
window.addEventListener("error", (e) => toasts.push("error", `Unexpected error: ${e.message}`));
window.addEventListener("unhandledrejection", (e) => toasts.push("error", `Unexpected error: ${String(e.reason)}`));

const app = mount(App, { target: document.getElementById("app")! });
export default app;
