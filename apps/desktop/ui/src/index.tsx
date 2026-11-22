import { render } from "solid-js/web";
import { tauriBridge } from "./bridge";
import { Home } from "./features/home/Home";
import "./styles/tokens.css";

const root = document.getElementById("root");
if (root) render(() => <Home bridge={tauriBridge} />, root);
