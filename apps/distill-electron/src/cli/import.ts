import { runImport } from "../distill/import";

function printHelp(): void {
  console.log("Usage: npm run import");
  console.log("");
  console.log("Discovers local Codex, Claude Code, and OpenCode captures and imports them into the Distill database.");
}

function main(): void {
  if (process.argv.includes("--help") || process.argv.includes("-h")) {
    printHelp();
    return;
  }

  const report = runImport();

  console.log(`Distill import at ${report.importedAt}`);
  console.log(`Database: ${report.databasePath}`);
  console.log(`Home: ${report.distillHome}\n`);

  for (const summary of report.sourceSummaries) {
    console.log(`${summary.kind}`);
    console.log(`  discovered: ${summary.discoveredCaptures}`);
    console.log(`  imported: ${summary.importedCaptures}`);
    console.log(`  skipped: ${summary.skippedCaptures}`);
    console.log(`  failed: ${summary.failedCaptures}\n`);
  }
}

main();
