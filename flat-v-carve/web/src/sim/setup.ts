// Simulator setup derived from a job and a plan result (U8 §3): tool
// normalization and the display-only stock rectangle. Pure module shared by
// the viewport component and the integration cross-check.
import type { Job } from '../contracts/job';
import type { Motion } from '../contracts/planning';
import type { SliceInfo } from '../contracts/stock';
import { chooseResolution } from './engine';
import type { Resolution, StockRect, ToolSpec } from './engine';

export interface SimSetup {
  stock: StockRect;
  tools: ToolSpec[];
  toolIds: string[];
  toolIndex: (toolId: string) => number | undefined;
  resolution: Resolution;
  detailMm: number;
}

/**
 * Stock rectangle per U8 §3: union of nominal target bounds, inflated by the
 * largest active tool radius plus 1 mm, snapped up to whole cells. Falls
 * back to the motion bounds when no slice reports a target.
 */
export function buildSimSetup(job: Job, motions: readonly Motion[], slices: readonly SliceInfo[]): SimSetup {
  const byId = new Map(job.tools.map(tool => [tool.id, tool]));
  const toolIds: string[] = [];
  const tools: ToolSpec[] = [];
  const add = (id: string) => {
    const tool = byId.get(id);
    if (!tool?.geometry) return undefined;
    const index = toolIds.indexOf(id);
    if (index !== -1) return index;
    if (tool.geometry.kind === 'endmill') {
      tools.push({ kind: 'endmill', diameterMm: tool.geometry.dimensions.diameter_mm });
    } else {
      tools.push({
        kind: 'vbit',
        includedAngleDeg: tool.geometry.dimensions.included_angle_deg,
        tipDiameterMm: tool.geometry.dimensions.tip_diameter_mm,
        maxCuttingDiameterMm: tool.geometry.dimensions.max_cutting_diameter_mm,
        cuttingHeightMm: tool.geometry.dimensions.cutting_height_mm,
      });
    }
    toolIds.push(id);
    return toolIds.length - 1;
  };
  add(job.operation.endmill_id);
  add(job.operation.vbit_id);
  if (tools.length === 0) throw new Error('The job has no tool geometry for the simulator.');
  const toolIndex = (toolId: string) => {
    const index = toolIds.indexOf(toolId);
    return index === -1 ? undefined : index;
  };
  for (const motion of motions) {
    if (toolIndex(motion.tool_id) === undefined) throw new Error(`Plan motion references unknown tool '${motion.tool_id}'.`);
  }

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const slice of slices) {
    const bounds = slice.regions.find(region => region.key === 'nominalTarget')?.bounds;
    if (!bounds) continue;
    minX = Math.min(minX, bounds.min.x);
    minY = Math.min(minY, bounds.min.y);
    maxX = Math.max(maxX, bounds.max.x);
    maxY = Math.max(maxY, bounds.max.y);
  }
  if (!Number.isFinite(minX)) {
    for (const motion of motions) {
      minX = Math.min(minX, motion.start.x, motion.end.x);
      minY = Math.min(minY, motion.start.y, motion.end.y);
      maxX = Math.max(maxX, motion.start.x, motion.end.x);
      maxY = Math.max(maxY, motion.start.y, motion.end.y);
    }
  }
  const radii = tools.map(tool => tool.kind === 'endmill'
    ? tool.diameterMm / 2
    : Math.max(tool.tipDiameterMm, tool.maxCuttingDiameterMm) / 2);
  const margin = Math.max(...radii, 0) + 1;
  const detail = Math.min(...tools.map(tool => tool.kind === 'endmill' ? tool.diameterMm : tool.tipDiameterMm));
  const resolution = chooseResolution({ x0: 0, y0: 0, x1: maxX - minX, y1: maxY - minY }, detail);
  const cols = Math.ceil((maxX - minX + 2 * margin) / resolution.cellMm);
  const rows = Math.ceil((maxY - minY + 2 * margin) / resolution.cellMm);
  const stock: StockRect = {
    x0: (minX + maxX) / 2 - cols * resolution.cellMm / 2,
    y0: (minY + maxY) / 2 - rows * resolution.cellMm / 2,
    x1: 0,
    y1: 0,
    thicknessMm: (() => {
      if (job.stock.thickness_mm === null || job.stock.thickness_mm <= 0) {
        throw new Error('The job has no stock thickness.');
      }
      return job.stock.thickness_mm;
    })(),
  };
  stock.x1 = stock.x0 + cols * resolution.cellMm;
  stock.y1 = stock.y0 + rows * resolution.cellMm;
  return { stock, tools, toolIds, toolIndex, resolution, detailMm: detail };
}
