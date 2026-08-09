/**
 * Integration tests for the App-Remover Application API (SPEC §7).
 *
 * Drives the real Express stack (createApp()) over loopback TCP and asserts the
 * documented contract: the §7.3 DTO shapes, the §7.4 error envelope, the §4.1 job
 * state machine, the FR-7 disambiguation flow (SPEC §7.6 Flow B), the FR-18 approve
 * guards, the FR-20 protected-app refusal, the FR-23 snapshot capture, the FR-33
 * append-only audit hash chain, and NFR-15 snapshot verification.
 */
import type { AddressInfo } from "node:net";
import type { Server } from "node:http";
import { createApp } from "../../src/app";

const VLC_DESKTOP = "/usr/share/applications/vlc.desktop";
const FIREFOX_DESKTOP = "/usr/share/applications/firefox.desktop";

let server: Server;
let base: string;

interface Resp {
  status: number;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  body: any;
}

async function req(method: string, path: string, body?: unknown): Promise<Resp> {
  const init: RequestInit = {
    method,
    headers: { "content-type": "application/json" },
  };
  if (body !== undefined) init.body = JSON.stringify(body);
  const res = await fetch(base + path, init);
  const text = await res.text();
  let parsed: unknown;
  try {
    parsed = text ? JSON.parse(text) : undefined;
  } catch {
    parsed = undefined;
  }
  return { status: res.status, body: parsed };
}

beforeAll(async () => {
  const app = createApp();
  server = app.listen(0, "127.0.0.1");
  await new Promise<void>((resolve) => server.once("listening", () => resolve()));
  const port = (server.address() as AddressInfo).port;
  base = `http://127.0.0.1:${port}/api/v1`;
});

afterAll(async () => {
  await new Promise<void>((resolve) => server.close(() => resolve()));
});

describe("Application API", () => {
  it("GET /health returns the HealthDTO envelope", async () => {
    const r = await req("GET", "/health");
    expect(r.status).toBe(200);
    expect(["ok", "degraded"]).toContain(r.body.status);
    expect(Array.isArray(r.body.backends)).toBe(true);
    expect(r.body.dbWritable).toBe(true);
  });

  it("GET /inventory lists the seeded apps (incl. two distinct Firefox)", async () => {
    const r = await req("GET", "/inventory");
    expect(r.status).toBe(200);
    expect(r.body.generatedAt).toBeTruthy();
    expect(Array.isArray(r.body.skippedBackends)).toBe(true);
    const names: string[] = r.body.items.map((i: { name: string }) => i.name);
    expect(names).toContain("vlc");
    expect(names).toContain("gnome-shell");
    expect(names.filter((n) => n === "firefox")).toHaveLength(2);
  });

  it("returns 404 NOT_FOUND for unknown routes", async () => {
    const r = await req("GET", "/does-not-exist");
    expect(r.status).toBe(404);
    expect(r.body.error.code).toBe("NOT_FOUND");
  });

  it("returns 400 VALIDATION_ERROR for malformed input (NFR-7)", async () => {
    const r = await req("POST", "/scans", { canonicalAppId: "not-a-uuid", scope: "both" });
    expect(r.status).toBe(400);
    expect(r.body.error.code).toBe("VALIDATION_ERROR");
    expect(Array.isArray(r.body.error.details.issues)).toBe(true);
  });

  it("refuses protected apps with 409 PROTECTED_APP (FR-20)", async () => {
    const inv = await req("GET", "/inventory");
    const gnome = inv.body.items.find((i: { name: string }) => i.name === "gnome-shell");
    const r = await req("POST", "/scans", { canonicalAppId: gnome.canonicalAppId, scope: "both" });
    expect(r.status).toBe(409);
    expect(r.body.error.code).toBe("PROTECTED_APP");
  });

  it("FR-7 disambiguation flow for two distinct Firefox instances (SPEC §7.6 Flow B)", async () => {
    const r = await req("POST", "/resolve", {
      source: { kind: "desktop-entry", path: FIREFOX_DESKTOP },
    });
    expect(r.status).toBe(200);
    expect(r.body.status).toBe("disambiguation-required");
    expect(r.body.candidates.length).toBe(2);
    expect(r.body.resolveToken).toBeTruthy();

    const chosen = r.body.candidates[0].canonicalAppId;
    const d = await req("POST", "/resolve/disambiguate", {
      resolveToken: r.body.resolveToken,
      selectedCanonicalAppIds: [chosen],
    });
    expect(d.status).toBe(200);
    expect(d.body.canonicalAppId).toBe(chosen);
    expect(d.body.instancesDisambiguated).toBe(true);
  });

  it("rejects purge-plan approval with a blocked op (409 PLAN_HAS_BLOCKED_OP, FR-18 AC1)", async () => {
    const inv = await req("GET", "/inventory");
    const vlc = inv.body.items.find(
      (i: { name: string; instanceCount: number }) => i.name === "vlc" && i.instanceCount === 1,
    );
    await req("POST", "/resolve", { source: { kind: "desktop-entry", path: VLC_DESKTOP } });
    const scan = await req("POST", "/scans", { canonicalAppId: vlc.canonicalAppId, scope: "both" });
    expect(scan.status).toBe(201);

    const purge = await req("POST", "/plans", {
      canonicalAppId: vlc.canonicalAppId,
      scanVersion: scan.body.scanVersion,
      mode: "purge",
      scope: "both",
    });
    expect(purge.status).toBe(201);
    expect(purge.body.blockedReasons.length).toBeGreaterThanOrEqual(1);

    const approve = await req("POST", `/plans/${purge.body.planId}/approve`, {
      acceptedRiskOperationIds: [],
    });
    expect(approve.status).toBe(409);
    expect(approve.body.error.code).toBe("PLAN_HAS_BLOCKED_OP");
  });

  it("end-to-end remove happy path: resolve → scan → plan → approve → job → audit → verify → undo", async () => {
    const inv = await req("GET", "/inventory");
    const vlc = inv.body.items.find(
      (i: { name: string; instanceCount: number }) => i.name === "vlc" && i.instanceCount === 1,
    );
    const vlcId: string = vlc.canonicalAppId;

    // Resolve (single instance → resolved, instancesDisambiguated true per O1).
    const resolved = await req("POST", "/resolve", {
      source: { kind: "desktop-entry", path: VLC_DESKTOP },
    });
    expect(resolved.status).toBe(200);
    expect(resolved.body.status).toBe("resolved");
    expect(resolved.body.application.instancesDisambiguated).toBe(true);

    // Scan → sealed graph.
    const scan = await req("POST", "/scans", { canonicalAppId: vlcId, scope: "both" });
    expect(scan.status).toBe(201);
    expect(typeof scan.body.scanVersion).toBe("number");
    expect(scan.body.counts).toBeDefined();
    const scanVersion: number = scan.body.scanVersion;

    // Dry Run → draft plan (remove mode ⇒ uninstall-package only, all safe).
    const plan = await req("POST", "/plans", {
      canonicalAppId: vlcId,
      scanVersion,
      mode: "remove",
      scope: "both",
    });
    expect(plan.status).toBe(201);
    expect(plan.body.status).toBe("draft");
    expect(plan.body.operations.length).toBeGreaterThanOrEqual(1);
    for (const op of plan.body.operations) {
      expect(op.action).toBe("uninstall-package");
      expect(op.verdict).toBe("safe");
    }
    const planId: string = plan.body.planId;

    // F3: creating a job from a DRAFT plan is refused at POST /jobs too.
    const jobFromDraft = await req("POST", "/jobs", { planId });
    expect(jobFromDraft.status).toBe(409);
    expect(jobFromDraft.body.error.code).toBe("PLAN_NOT_APPROVED");

    // Approve.
    const approve = await req("POST", `/plans/${planId}/approve`, {
      acceptedRiskOperationIds: [],
    });
    expect(approve.status).toBe(200);
    expect(approve.body.status).toBe("approved");

    // Create job → snapshot captured before leaving 'created' (FR-23 AC1).
    const job = await req("POST", "/jobs", { planId });
    expect(job.status).toBe(201);
    expect(job.body.status).toBe("created");
    expect(job.body.snapshotId).not.toBeNull();
    const jobId: string = job.body.jobId;
    const snapshotId: string = job.body.snapshotId;

    // Begin → executes to terminal 'completed'.
    const begin = await req("POST", `/jobs/${jobId}/begin`);
    expect(begin.status).toBe(200);
    expect(begin.body.status).toBe("completed");
    for (const step of begin.body.steps) {
      expect(step.status).toBe("succeeded");
    }

    // FR-26 AC1: begin again from a terminal state is illegal.
    const beginAgain = await req("POST", `/jobs/${jobId}/begin`);
    expect(beginAgain.status).toBe(409);
    expect(beginAgain.body.error.code).toBe("ILLEGAL_TRANSITION");

    // History + audit (genesis record ⇒ prevHash null, hash 64 chars).
    const history = await req("GET", "/history");
    expect(history.status).toBe(200);
    const item = history.body.items.find((i: { jobId: string }) => i.jobId === jobId);
    expect(item).toBeDefined();
    expect(item.outcome).toBe("completed");
    expect(item.undoable).toBe(true);

    const audit = await req("GET", `/audit/${item.auditRecordId}`);
    expect(audit.status).toBe(200);
    expect(audit.body.hash).toHaveLength(64);
    expect(audit.body.prevHash).toBeNull(); // genesis record (NFR-9)
    expect(audit.body.snapshotId).toBe(snapshotId);

    // NFR-15: snapshot integrity verify.
    const verify = await req("POST", `/snapshots/${snapshotId}/verify`);
    expect(verify.status).toBe(201);
    expect(verify.body.intact).toBe(true);
    expect(verify.body.failures).toEqual([]);

    // FR-34: undo restores (snapshot intact ⇒ no SNAPSHOT_CORRUPT).
    const undo = await req("POST", `/audit/${item.auditRecordId}/undo`);
    expect(undo.status).toBe(201);
    expect(undo.body.restoreJobId).toBeTruthy();
    expect(Array.isArray(undo.body.deferredPackages)).toBe(true);
  });
});
