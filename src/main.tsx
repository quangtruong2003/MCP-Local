import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";
import "./styles/tokens.css";
import "./styles/global.css";
import "./styles/components.css";
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode><App /></React.StrictMode>
);
