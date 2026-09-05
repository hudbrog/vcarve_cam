import type { Draft } from './draft';

export interface WorkspaceState {
  draft: Draft; revision: number; past: Draft[]; future: Draft[];
  editStart: Draft | null; downloadedRevision: number | null;
}
export type WorkspaceAction =
  | { type: 'text'; path: string; value: string }
  | { type: 'commit' }
  | { type: 'replace'; draft: Draft }
  | { type: 'name'; value: string }
  | { type: 'include'; ids: string[] }
  | { type: 'clear-fields'; paths: string[] }
  | { type: 'undo' | 'redo' | 'downloaded' };
export const initialWorkspace = (draft: Draft): WorkspaceState => ({ draft, revision: 0, past: [], future: [], editStart: null, downloadedRevision: null });
function commit(state: WorkspaceState): WorkspaceState {
  if (!state.editStart) return state;
  return { ...state, past: [...state.past.slice(-49), state.editStart], editStart: null };
}
export function workspaceReducer(state: WorkspaceState, action: WorkspaceAction): WorkspaceState {
  switch (action.type) {
    case 'text':
      if (state.draft.text[action.path] === action.value) return state;
      return { ...state, editStart: state.editStart ?? state.draft, future: [], revision: state.revision + 1,
        draft: { ...state.draft, text: { ...state.draft.text, [action.path]: action.value } } };
    case 'name':
      if (state.draft.base.name === action.value) return state;
      return { ...state, editStart: state.editStart ?? state.draft, future: [], revision: state.revision + 1,
        draft: { ...state.draft, base: { ...state.draft.base, name: action.value } } };
    case 'commit': return commit(state);
    case 'replace': {
      const current = commit(state);
      return { ...current, draft: action.draft, revision: state.revision + 1,
        past: [...current.past.slice(-49), current.draft], future: [], downloadedRevision: null };
    }
    case 'include':
      return workspaceReducer(state, { type: 'replace', draft: { ...state.draft,
        base: { ...state.draft.base, selected_region_ids: action.ids } } });
    case 'clear-fields':
      return workspaceReducer(state, { type: 'replace', draft: { ...state.draft,
        text: { ...state.draft.text, ...Object.fromEntries(action.paths.map(path => [path, ''])) } } });
    case 'undo': {
      const current = commit(state);
      const previous = current.past.at(-1);
      if (!previous) return current;
      return { ...current, draft: previous, past: current.past.slice(0, -1), future: [...current.future, current.draft], revision: state.revision + 1 };
    }
    case 'redo': {
      const next = state.future.at(-1);
      if (!next) return state;
      return { ...state, draft: next, past: [...state.past.slice(-49), state.draft], future: state.future.slice(0, -1), editStart: null, revision: state.revision + 1 };
    }
    case 'downloaded': return { ...state, downloadedRevision: state.revision };
  }
}
