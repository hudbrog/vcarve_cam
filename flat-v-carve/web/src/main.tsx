import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { createHttpService } from './service/http';
import { fixtureService } from './service/fixture';
import { createWasmService } from './service/wasm';
import { createAutoService } from './service/auto';
import './styles.css';

// Fixture mode keeps development offline; ?mode= overrides detection between
// the local portable service and the statically hostable in-browser engine.
const mode = new URLSearchParams(location.search).get('mode');
const service = import.meta.env.DEV || mode === 'fixture' ? fixtureService
  : mode === 'wasm' ? createWasmService()
  : mode === 'live' ? createHttpService()
  : createAutoService();
createRoot(document.getElementById('root')!).render(<StrictMode><App service={service} /></StrictMode>);
