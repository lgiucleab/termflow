import "@fontsource-variable/jetbrains-mono";
import "@xterm/xterm/css/xterm.css";
import "./styles/tokens.css";
import "./styles/app.css";
import { render } from "solid-js/web";
import { App } from "./App";
import { tauriBridge } from "./bridge";

const root = document.getElementById("root");
if (root) render(() => <App bridge={tauriBridge} />, root);
