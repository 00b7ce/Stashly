import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { PrivacyGate } from "./privacy";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <PrivacyGate>
      <App />
    </PrivacyGate>
  </React.StrictMode>,
);
