import path from "node:path";
import { app, BrowserWindow, ipcMain } from "electron";
import { browseDbTable, getDbExplorerSnapshot, runDbQuery } from "../distill/db_inspector";
import {
  enqueueSourceSyncJob,
  getBackgroundSyncStatus,
  markStaleRunningSyncJobsFailed,
  runNextSourceSyncJob
} from "../distill/jobs";
import { BACKGROUND_SYNC_INTERVAL_MINUTES } from "../distill/settings";

const BACKGROUND_SYNC_INTERVAL_MS = BACKGROUND_SYNC_INTERVAL_MINUTES * 60 * 1000;

let mainWindow: BrowserWindow | null = null;
let backgroundSyncTimer: NodeJS.Timeout | undefined;
let isSyncing = false;

function createMainWindow(): void {
  mainWindow = new BrowserWindow({
    width: 1120,
    height: 760,
    minWidth: 900,
    minHeight: 620,
    backgroundColor: "#1a1a1e",
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false
    }
  });

  mainWindow.maximize();
  mainWindow.loadFile(path.join(app.getAppPath(), "static", "index.html"));
}

function broadcastBackgroundSyncStatus(): void {
  const status = getBackgroundSyncStatus();
  mainWindow?.webContents.send("distill:background-sync", status);
}

function runBackgroundSync(reason: string): void {
  if (isSyncing) {
    return;
  }

  isSyncing = true;

  try {
    enqueueSourceSyncJob(reason);
    broadcastBackgroundSyncStatus();
    const status = runNextSourceSyncJob();
    console.log(status.summary);
    mainWindow?.webContents.send("distill:background-sync", status);
  } finally {
    isSyncing = false;
  }
}

function startBackgroundSyncLoop(): void {
  backgroundSyncTimer = setInterval(() => {
    runBackgroundSync("interval");
  }, BACKGROUND_SYNC_INTERVAL_MS);
}

ipcMain.handle("distill:get-background-sync-status", () => getBackgroundSyncStatus());
ipcMain.handle("distill:request-background-sync", () => {
  runBackgroundSync("manual");
  return getBackgroundSyncStatus();
});
ipcMain.handle("distill:get-db-explorer-snapshot", () => getDbExplorerSnapshot());
ipcMain.handle("distill:browse-db-table", (_event, request) => browseDbTable(request));
ipcMain.handle("distill:run-db-query", (_event, request) => runDbQuery(request));

app.whenReady().then(() => {
  markStaleRunningSyncJobsFailed();
  runBackgroundSync("startup");
  createMainWindow();
  startBackgroundSyncLoop();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createMainWindow();
    }
  });
});

app.on("window-all-closed", () => {
  if (backgroundSyncTimer) {
    clearInterval(backgroundSyncTimer);
    backgroundSyncTimer = undefined;
  }

  if (process.platform !== "darwin") {
    app.quit();
  }
});
