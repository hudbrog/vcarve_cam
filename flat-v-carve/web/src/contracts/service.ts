import type { Job, Point } from './job';

// Proposed UI API, deliberately separate from portable job/plan schemas.
// A production implementation must validate wire DTOs and engine identities.
export interface Capabilities {
  apiVersion: 'ui-proposal-1';
  mode: 'fixture' | 'live';
  engineVersion: string;
  importArtwork: boolean;
  validateDraft: boolean;
  planningStages: ('endmill' | 'combined')[];
  verificationScopes: string[];
  exportFormats: string[];
}
export interface DisplayComponent {
  id: string;
  label: string;
  rings: { hole: boolean; points: Point[] }[];
}
export interface ArtworkDisplay {
  coordinateSpace: 'source-page-mm-y-up';
  widthMm: number;
  heightMm: number;
  components: DisplayComponent[];
  engineVersion: string;
  geometryToleranceMm: number;
  description: string;
}
export interface Diagnostic {
  code: string;
  severity: 'info' | 'warning' | 'error';
  message: string;
  fieldPath?: string;
  sourceId?: string;
}
export interface CamService {
  capabilities(signal?: AbortSignal): Promise<Capabilities>;
  openExample(signal?: AbortSignal): Promise<{ job: Job; display: ArtworkDisplay }>;
  displayFor(job: Job, signal?: AbortSignal): Promise<ArtworkDisplay | null>;
  validateDraft(job: Job, revision: number, signal?: AbortSignal): Promise<{
    revision: number; diagnostics: Diagnostic[]; authoritative: boolean;
  }>;
}

export function outputBlockedReasons(capabilities: Capabilities | null): string[] {
  if (!capabilities) return ['The local service is not connected.'];
  const reasons: string[] = [];
  if (capabilities.mode === 'fixture') reasons.push('Fixture mode cannot create machine programs.');
  if (!capabilities.verificationScopes.includes('continuous-stock')) reasons.push('Required geometric verification is unavailable.');
  if (!capabilities.exportFormats.includes('linuxcnc')) reasons.push('LinuxCNC generation and formatted-motion checks are unavailable.');
  // U1 has no engine-issued plan/output identity, even if an adapter advertises output.
  reasons.push('No current, independently verified plan and checked output are loaded.');
  return reasons;
}
