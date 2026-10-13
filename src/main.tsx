import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import "./styles.css";
import { platform } from "./api";

// Each platform gets its own native styling (see the end of styles.css).
document.documentElement.dataset.platform = platform;

// When the window's native material is active (macOS vibrancy, Windows 11 Mica),
// let it show through instead of painting a solid background.
invoke<boolean>("native_effects")
  .then((on) => {
    if (on) document.documentElement.dataset.effects = "on";
  })
  .catch(() => {});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
