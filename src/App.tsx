// The root: which window are we, and therefore which UI do we render.

import { useState } from "react";
import { MainWindow } from "./components/MainWindow";
import { Overlay } from "./components/Overlay";
import { resolveWindowLabel } from "./lib/window";
import "./App.css";

function App() {
  // The label is fixed for the lifetime of the webview, so it is derived once
  // during the first render rather than pushed in through an effect (which
  // would cause a cascading render on every mount).
  const [windowLabel] = useState<string>(resolveWindowLabel);

  if (windowLabel === "overlay") {
    return <Overlay />;
  }
  return <MainWindow />;
}

export default App;
