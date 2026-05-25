import { exportApprovedSessions } from "../distill/export";

function printHelp(): void {
  console.log("Usage: npm run export -- [dataset]");
  console.log("");
  console.log("Exports approved dataset sessions to JSONL.");
  console.log("Defaults to the \"train\" dataset when no dataset is provided.");
}

function main(): void {
  if (process.argv.includes("--help") || process.argv.includes("-h")) {
    printHelp();
    return;
  }

  const dataset = process.argv[2] ?? "train";
  const report = exportApprovedSessions(dataset);

  console.log(`Distill export at ${report.exportedAt}`);
  console.log(`Dataset: ${report.dataset}`);
  console.log(`Output: ${report.outputPath}`);
  console.log(`Records: ${report.recordCount}`);
}

main();
