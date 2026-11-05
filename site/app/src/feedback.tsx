import { useEffect, useRef } from "react";
import { mountFeedback } from "../../../feedback/client/form.mjs";
import { feedback } from "./content";

export function FeedbackForm() {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (ref.current)
      return mountFeedback(ref.current, {
        source: "site",
        version: feedback.version,
        commit: feedback.commit,
      });
  }, []);
  return <div ref={ref} />;
}
