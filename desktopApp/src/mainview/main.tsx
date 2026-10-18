import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import './index.css'
import { Electroview } from "electrobun/view";

const params = new URLSearchParams(window.location.search);

if (import.meta.env.DEV && params.has('mock')) {
  // Browser preview without a phone or the Rust library: see src/dev/mockBridge.ts.
  const { installMockBridge } = await import('@/dev/mockBridge');
  installMockBridge((params.get('mock') || 'none') as Parameters<typeof installMockBridge>[0]);
} else {
  // Initialize the Electrobun bridge for RPC and messaging
  const rpc = Electroview.defineRPC({
    // The default is 1 s. The bridge reports ready before the backend has
    // finished `native.init()`, so the startup checks used to time out, fall
    // back to "nothing installed", and hide the player and OBS buttons.
    maxRequestTime: 10_000,
    handlers: {
      requests: {
        // These would be requests FROM Bun TO View (currently unused)
      },
    },
  });

  // Constructing the view installs the RPC transport as a side effect.
  new Electroview({ rpc });

  // Export rpc so App can use it for polling
  (window as any).__mirrorRpc = rpc;
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
