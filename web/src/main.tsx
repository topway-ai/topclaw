/**
 * Browser entrypoint for the embedded TopClaw dashboard.
 *
 * The SPA is served by the Rust gateway, but client-side routes remain rooted
 * at `/` because the backend provides an SPA fallback for dashboard pages.
 */
import React from 'react';
import ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import './index.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {/* Vite base '/_app/' scopes static asset URLs only; app routes stay rooted at '/' for SPA fallback. */}
    <BrowserRouter basename="/">
      <App />
    </BrowserRouter>
  </React.StrictMode>
);
