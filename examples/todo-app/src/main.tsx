import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App.tsx' // Changed from .js/ts to explicit .tsx for clarity

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
