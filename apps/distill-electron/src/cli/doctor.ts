import { buildDoctorReport } from "../distill/doctor";
import { DiscoveredSource } from "../shared/types";

function printHelp(): void {
  console.log("Usage: npm run doctor");
  console.log("");
  console.log("Reports install and data-root health for the supported local sources.");
}

function formatSource(source: DiscoveredSource): string {
  const lines = [
    `${source.displayName} [${source.kind}]`,
    `  status: ${source.installStatus}`,
    `  executable: ${source.executablePath ?? "not found"}`,
    `  data root: ${source.dataRoot ?? "unknown"}`
  ];

  for (const check of source.checks) {
    const countPart =
      typeof check.fileCount === "number" ? ` (${check.fileCount} files)` : "";
    lines.push(`  ${check.label}: ${check.exists ? "found" : "missing"} ${check.path}${countPart}`);
  }

  return lines.join("\n");
}

function main(): void {
  if (process.argv.includes("--help") || process.argv.includes("-h")) {
    printHelp();
    return;
  }

  const report = buildDoctorReport();

  console.log(`Distill doctor scan at ${report.scannedAt}\n`);
  for (const source of report.sources) {
    console.log(formatSource(source));
    console.log("");
  }
}

main();
