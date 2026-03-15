import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App.tsx'
import './index.css'
import { Electroview } from "electrobun/view";

// Initialize the Electrobun bridge for RPC and messaging
const rpc = Electroview.defineRPC({
    role: 'view',
    handlers: {
        requests: {
            // These would be requests FROM Bun TO View (currently unused)
        }
    }
});

const electroview = new Electroview({ rpc });

// Export rpc so App can use it for polling
(window as any).__mirrorRpc = rpc;

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
