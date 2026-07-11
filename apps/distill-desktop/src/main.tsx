/**
 * Renderer entrypoint. Mounts the first-run UI with the Tauri bridge.
 */

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { createTauriBridge } from "./bridge";
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

createRoot(root).render(
  <StrictMode>
    <App bridge={createTauriBridge()} />
  </StrictMode>,
);
