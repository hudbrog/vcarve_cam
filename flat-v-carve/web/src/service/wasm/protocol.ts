// Wire types shared by the wasm parent worker, its compute children, and the
// main-thread service. The task/snapshot shapes are the same ones the local
// HTTP service emits; see src/contracts for the authoritative zod schemas.

export interface WasmFailure {
  status: number;
  code: string;
  message: string;
}
export type WasmReply<T = unknown> = { ok: T } | { error: WasmFailure };

export interface WasmLimits {
  svgBytes: number;
  jobBytes: number;
  requestBytes: number;
  pageMotions: number;
  reportBytes: number;
  profileBytes: number;
  programBytes: number;
  sliceVertices: number;
  inspectionVertices: number;
}

export interface WasmInit {
  engineVersion: string;
  instanceId: string;
  sessionToken: string;
  limits: WasmLimits;
  defaultVerificationOptions: unknown;
}

// Main thread -> parent worker RPC.
export interface WasmRequest {
  id: string;
  op: string;
  payload?: unknown;
}
export type WasmResponse = { id: string } & WasmReply;
