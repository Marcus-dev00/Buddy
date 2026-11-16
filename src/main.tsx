import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import SnipOverlay from "./components/SnipOverlay";
import "./index.css";

const isSnipWindow = window.location.hash === "#snip";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isSnipWindow ? <SnipOverlay /> : <App />}
  </React.StrictMode>
);
