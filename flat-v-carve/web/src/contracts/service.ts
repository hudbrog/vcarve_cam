import type { Job, Point } from './job';
import type { PlanningLimits, PlanTask, PlanResult, TaskIdentity } from './planning';

// Versioned local UI API, deliberately separate from portable job/plan schemas.
export interface Capabilities {
  apiVersion: 'ui-2';
  mode: 'fixture' | 'live';
  engineVersion: string;
  importArtwork: boolean;
  openJob: boolean;
  validateDraft: boolean;
  planningStages: ('endmill' | 'combined')[];
  verificationScopes: string[];
  exportFormats: string[];
  limits?: { svgBytes: number; jobBytes: number; requestBytes: number; concurrentInspections: number };
  planning?: PlanningLimits;
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
  stage?: string;
}
export interface Validation {
  revision: number; diagnostics: Diagnostic[]; authoritative: boolean;
  valid?: boolean; scope?: 'editable-job-and-svg'; missingMachiningFields?: string[];
  documentFingerprint?: string | null;
}
export interface OpenedDocument {
  job: Job; display: ArtworkDisplay; diagnostics: Diagnostic[];
  missingMachiningFields: string[]; documentFingerprint: string;
}
export function editableDownloadAllowed(validation: Validation | undefined, revision: number): boolean {
  return validation?.authoritative === true && validation.valid === true
    && validation.revision === revision && validation.scope === 'editable-job-and-svg'
    && !!validation.documentFingerprint;
}
export interface CamService {
  capabilities(signal?: AbortSignal): Promise<Capabilities>;
  openExample(signal?: AbortSignal): Promise<{ job: Job; display: ArtworkDisplay }>;
  displayFor(job: Job, signal?: AbortSignal): Promise<ArtworkDisplay | null>;
  validateDraft(job: Job, revision: number, signal?: AbortSignal): Promise<Validation>;
  openJob?(json: string, revision: number, signal?: AbortSignal): Promise<OpenedDocument>;
  importArtwork?(filename: string, svg: string, options: Job['import'], revision: number, signal?: AbortSignal): Promise<OpenedDocument>;
  startPlan?(job: Job, identity: TaskIdentity, signal?: AbortSignal): Promise<PlanTask>;
  planTask?(identity: TaskIdentity, signal?: AbortSignal): Promise<PlanTask>;
  cancelPlan?(identity: TaskIdentity, signal?: AbortSignal): Promise<PlanTask>;
  planResult?(identity: TaskIdentity, signal?: AbortSignal): Promise<PlanResult>;
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
