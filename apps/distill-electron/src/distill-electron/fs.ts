import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";

export function expandHome(inputPath: string): string {
  if (!inputPath.startsWith("~/")) {
    return inputPath;
  }

  const home = process.env.HOME;
  if (!home) {
    return inputPath;
  }

  return path.join(home, inputPath.slice(2));
}

export function pathExists(targetPath: string): boolean {
  return fs.existsSync(targetPath);
}

export function countFiles(targetPath: string): number {
  if (!fs.existsSync(targetPath)) {
    return 0;
  }

  const stat = fs.statSync(targetPath);
  if (stat.isFile()) {
    return 1;
  }

  let total = 0;
  for (const entry of fs.readdirSync(targetPath, { withFileTypes: true })) {
    const childPath = path.join(targetPath, entry.name);
    if (entry.isDirectory()) {
      total += countFiles(childPath);
    } else {
      total += 1;
    }
  }

  return total;
}

export function countFilesMatching(targetPath: string, predicate: (filePath: string) => boolean): number {
  return listFilesRecursive(targetPath).filter(predicate).length;
}

export function findExecutable(binaryName: string): string | undefined {
  const envPath = process.env.PATH;
  if (!envPath) {
    return undefined;
  }

  const accessMode = process.platform === "win32" ? fs.constants.F_OK : fs.constants.X_OK;
  const pathExts = process.platform === "win32"
    ? (process.env.PATHEXT ?? ".COM;.EXE;.BAT;.CMD")
      .split(";")
      .filter(Boolean)
      .map((ext) => ext.toLowerCase())
    : [];
  const candidateNames = path.extname(binaryName) || process.platform !== "win32"
    ? [binaryName]
    : [binaryName, ...pathExts.map((ext) => `${binaryName}${ext}`)];

  for (const part of envPath.split(path.delimiter)) {
    for (const candidateName of candidateNames) {
      const candidate = path.join(part, candidateName);
      try {
        if (!fs.statSync(candidate).isFile()) {
          continue;
        }

        fs.accessSync(candidate, accessMode);
        return candidate;
      } catch {
        continue;
      }
    }
  }

  return undefined;
}

export function ensureDirectory(targetPath: string): void {
  fs.mkdirSync(targetPath, { recursive: true });
}

export function listFilesRecursive(targetPath: string): string[] {
  if (!fs.existsSync(targetPath)) {
    return [];
  }

  const stat = fs.statSync(targetPath);
  if (stat.isFile()) {
    return [targetPath];
  }

  const results: string[] = [];
  for (const entry of fs.readdirSync(targetPath, { withFileTypes: true })) {
    const childPath = path.join(targetPath, entry.name);
    if (entry.isDirectory()) {
      results.push(...listFilesRecursive(childPath));
    } else {
      results.push(childPath);
    }
  }

  return results.sort();
}

export function getFileSha256(filePath: string): string {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(filePath));
  return hash.digest("hex");
}

export function getTextSha256(text: string): string {
  const hash = crypto.createHash("sha256");
  hash.update(text);
  return hash.digest("hex");
}

export function getTextSha1(text: string): string {
  const hash = crypto.createHash("sha1");
  hash.update(text);
  return hash.digest("hex");
}
