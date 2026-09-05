import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { createHttpService } from './service/http';
import { fixtureService } from './service/fixture';
import './styles.css';

const service = import.meta.env.DEV || new URLSearchParams(location.search).get('mode') === 'fixture' ? fixtureService : createHttpService();
createRoot(document.getElementById('root')!).render(<StrictMode><App service={service} /></StrictMode>);
