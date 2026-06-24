import { useEffect, useState } from "react";

export function useStickyQueryParam(name: string) {
  const [value, setValueState] = useState(() => queryParam(name));

  useEffect(() => {
    function sync() {
      setValueState(queryParam(name));
    }
    window.addEventListener("popstate", sync);
    return () => window.removeEventListener("popstate", sync);
  }, [name]);

  function setValue(next: string | null, mode: "push" | "replace" = "push") {
    writeQueryParam(name, next, mode);
    setValueState(next);
  }

  return [value, setValue] as const;
}

export function queryParam(name: string) {
  const value = new URLSearchParams(window.location.search).get(name);
  return value?.trim() || null;
}

export function writeQueryParam(
  name: string,
  value: string | null,
  mode: "push" | "replace" = "push",
) {
  const url = new URL(window.location.href);
  if (value) url.searchParams.set(name, value);
  else url.searchParams.delete(name);
  const target = `${url.pathname}${url.search}${url.hash}`;
  if (
    target ===
    `${window.location.pathname}${window.location.search}${window.location.hash}`
  )
    return;
  if (mode === "push") window.history.pushState(null, "", target);
  else window.history.replaceState(null, "", target);
}
