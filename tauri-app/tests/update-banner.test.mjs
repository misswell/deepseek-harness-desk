import assert from "assert";
import { shouldShowAppUpdateBanner } from "../src/update-banner.js";

const update = {
  available: true,
  latest_version: "0.3.26",
};

assert.equal(
  shouldShowAppUpdateBanner(update, null, "0.3.26"),
  false,
  "closing the banner must keep the current version hidden",
);

console.log("✓ dismissed app update stays hidden when the same update is rendered again");

assert.equal(
  shouldShowAppUpdateBanner({ ...update, latest_version: "0.3.27" }, null, "0.3.26"),
  true,
  "a newer version must not inherit the previous dismissal",
);
assert.equal(
  shouldShowAppUpdateBanner(update, "0.3.26", null),
  false,
  "ignoring a version must keep it hidden",
);

console.log("✓ newer versions reappear and ignored versions stay hidden");
