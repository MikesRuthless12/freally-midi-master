import { useEffect, useState } from "react";
import reactLogo from "./assets/react.svg";
import { invoke } from "@tauri-apps/api/core";
import {
  BugReportOverlay,
  bugReportHasPendingCrash,
} from "./components/BugReport/BugReport";
import "./App.css";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");
  const [bugReportOpen, setBugReportOpen] = useState(false);

  // A crash left a report behind: the relaunched app opens it on its own, which
  // is the whole point of the crash loop. A pending crash takes the dialog slot
  // ahead of anything else that wants it at launch.
  useEffect(() => {
    bugReportHasPendingCrash()
      .then((pending) => {
        if (pending) setBugReportOpen(true);
      })
      .catch(() => {
        /* No backend (plain `vite dev`) — nothing to surface. */
      });
  }, []);

  async function greet() {
    // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
    setGreetMsg(await invoke("greet", { name }));
  }

  return (
    <main className="container">
      <h1>Welcome to Tauri + React</h1>

      <div className="row">
        <a href="https://vite.dev" target="_blank">
          <img src="/vite.svg" className="logo vite" alt="Vite logo" />
        </a>
        <a href="https://tauri.app" target="_blank">
          <img src="/tauri.svg" className="logo tauri" alt="Tauri logo" />
        </a>
        <a href="https://react.dev" target="_blank">
          <img src={reactLogo} className="logo react" alt="React logo" />
        </a>
      </div>
      <p>Click on the Tauri, Vite, and React logos to learn more.</p>

      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          greet();
        }}
      >
        <input
          id="greet-input"
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder="Enter a name..."
        />
        <button type="submit">Greet</button>
      </form>
      <p>{greetMsg}</p>

      <div className="row">
        <button type="button" onClick={() => setBugReportOpen(true)}>
          Report a bug
        </button>
      </div>

      {bugReportOpen && <BugReportOverlay onClose={() => setBugReportOpen(false)} />}
    </main>
  );
}

export default App;
