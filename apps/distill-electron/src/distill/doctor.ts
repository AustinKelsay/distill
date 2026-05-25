import { sourceConnectors } from "../connectors";
import { DoctorReport } from "../shared/types";

export function buildDoctorReport(): DoctorReport {
  return {
    scannedAt: new Date().toISOString(),
    sources: sourceConnectors.map((connector) => connector.detect())
  };
}
