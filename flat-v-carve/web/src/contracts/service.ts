import type { LibraryConnection, LibrarySnapshot, LibraryChange, LibrarySelection, LibraryCapture, LibraryCandidate } from './library';
import type { ExportIdentity, ExportTask, ExportResult } from './export';
import type { Job, Point } from './job';
import type { PlanningLimits, PlanTask, PlanResult, TaskIdentity, SliceResponse } from './planning';
import type { SliceInfo } from './stock';
import type { VerificationOptions } from './verificationOptions';
import type { VerificationIdentity, VerificationTask, VerificationResult } from './verification';

// Versioned local UI API, deliberately separate from portable job/plan schemas.
export interface Capabilities {
  apiVersion: 'ui-7';
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
  verification?: { defaultOptions: VerificationOptions };
  toolLibrary?: {schemaVersion:1;maxBytes:number;maxTools:number;maxPresetsPerTool:number;location:string}|null;
  export?: { profileBytes: number; programBytes: number; layouts: ('combined' | 'per_tool')[] };
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
  library?(connection:LibraryConnection,signal?:AbortSignal):Promise<LibrarySnapshot>;
  initializeLibrary?(connection:LibraryConnection,signal?:AbortSignal):Promise<LibrarySnapshot>;
  changeLibrary?(connection:LibraryConnection,revision:number,change:LibraryChange,signal?:AbortSignal):Promise<LibrarySnapshot>;
  importLibrary?(connection:LibraryConnection,revision:number,json:string,signal?:AbortSignal):Promise<LibrarySnapshot>;
  captureLibraryTool?(connection:LibraryConnection,input:LibraryCapture,signal?:AbortSignal):Promise<LibrarySnapshot>;
  applyLibraryTool?(connection:LibraryConnection,input:LibrarySelection,signal?:AbortSignal):Promise<LibraryCandidate>;
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
  stockSlice?(identity: TaskIdentity, slice: SliceInfo, signal?: AbortSignal): Promise<SliceResponse>;
  startExport?(identity: ExportIdentity, signal?: AbortSignal): Promise<ExportTask>;
  exportTask?(identity: ExportIdentity, signal?: AbortSignal): Promise<ExportTask>;
  cancelExport?(identity: ExportIdentity, signal?: AbortSignal): Promise<ExportTask>;
  exportResult?(identity: ExportIdentity, signal?: AbortSignal): Promise<ExportResult>;
  startVerification?(identity: VerificationIdentity, signal?: AbortSignal): Promise<VerificationTask>;
  verificationTask?(identity: VerificationIdentity, signal?: AbortSignal): Promise<VerificationTask>;
  cancelVerification?(identity: VerificationIdentity, signal?: AbortSignal): Promise<VerificationTask>;
  verificationResult?(identity: VerificationIdentity, signal?: AbortSignal): Promise<VerificationResult>;
}

export function outputBlockedReasons(capabilities: Capabilities | null): string[] {
  if (!capabilities) return ['The local service is not connected.'];
  const reasons: string[] = [];
  if (capabilities.mode === 'fixture') reasons.push('Fixture mode cannot create machine programs.');
  if (!capabilities.verificationScopes.includes('continuous-stock')) reasons.push('Required geometric verification is unavailable.');
  if (!capabilities.exportFormats.includes('linuxcnc')) reasons.push('LinuxCNC generation and formatted-motion checks are unavailable.');

  return reasons;
}
