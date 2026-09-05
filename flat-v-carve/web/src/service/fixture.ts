import capturedJob from '../fixtures/inkscape.job.json';
import capturedDisplay from '../fixtures/inkscape.display.json';
import { parseJob, type Job } from '../contracts/job';
import type { ArtworkDisplay, CamService } from '../contracts/service';

const example = parseJob(capturedJob);
const display: ArtworkDisplay = {
  ...capturedDisplay,
  coordinateSpace: 'source-page-mm-y-up',
};

function check(signal?: AbortSignal) { signal?.throwIfAborted(); }
export function fixtureDisplayMatches(job: Job) {
  return job.source.svg === example.source.svg
    && job.import.geometry_tolerance_mm === example.import.geometry_tolerance_mm
    && job.import.ticks_per_mm === example.import.ticks_per_mm;
}

export const fixtureService: CamService = {
  async capabilities(signal) {
    check(signal);
    return { apiVersion: 'ui-proposal-1', mode: 'fixture', engineVersion: display.engineVersion,
      importArtwork: false, validateDraft: false, planningStages: [], verificationScopes: [], exportFormats: [] };
  },
  async openExample(signal) {
    check(signal);
    return { job: structuredClone(example), display: structuredClone(display) };
  },
  async displayFor(job, signal) {
    check(signal);
    return fixtureDisplayMatches(job) ? structuredClone(display) : null;
  },
  async validateDraft(_job, revision, signal) {
    check(signal);
    return { revision, authoritative: false, diagnostics: [{
      code: 'ENGINE_VALIDATION_UNAVAILABLE', severity: 'info',
      message: 'Draft settings need validation by the local Rust service. Fixture mode supplies artwork display only.',
    }] };
  },
};
