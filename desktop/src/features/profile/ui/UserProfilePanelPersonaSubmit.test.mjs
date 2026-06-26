import assert from "node:assert/strict";
import test from "node:test";

import {
  submitProfilePersonaDialog,
  validateLinkedAgentRuntimeEdit,
} from "./UserProfilePanelPersonaSubmit.ts";

function agent(overrides = {}) {
  return {
    pubkey: "deadbeef".repeat(8),
    name: "Fizz",
    personaId: "persona-1",
    relayUrl: "ws://localhost:3000",
    acpCommand: "buzz-acp",
    agentCommand: "goose",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 320,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: "Prompt",
    avatarUrl: null,
    model: null,
    mcpToolsets: null,
    envVars: {},
    status: "stopped",
    pid: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    logPath: null,
    startOnAppLaunch: true,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

function persona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Fizz",
    avatarUrl: null,
    systemPrompt: "Prompt",
    runtime: "goose",
    model: null,
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    envVars: {},
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function updateInput(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Fizz",
    avatarUrl: undefined,
    systemPrompt: "Prompt",
    runtime: "claude",
    model: undefined,
    provider: undefined,
    namePool: [],
    ...overrides,
  };
}

function createInput(overrides = {}) {
  return {
    displayName: "Fizz",
    avatarUrl: "",
    systemPrompt: "Prompt",
    runtime: "goose",
    model: undefined,
    provider: undefined,
    namePool: [],
    envVars: {},
    ...overrides,
  };
}

function runtime(overrides = {}) {
  return {
    id: "claude",
    label: "Claude Code",
    avatarUrl: "",
    availability: "available",
    command: "claude",
    binaryPath: "/usr/local/bin/claude",
    defaultArgs: [],
    mcpCommand: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
    ...overrides,
  };
}

test("validateLinkedAgentRuntimeEdit allows available runtime changes", () => {
  assert.equal(
    validateLinkedAgentRuntimeEdit({
      input: updateInput({ runtime: "claude" }),
      managedAgent: agent(),
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [runtime()],
    }),
    null,
  );
});

test("validateLinkedAgentRuntimeEdit rejects unavailable linked-agent runtime changes", () => {
  assert.equal(
    validateLinkedAgentRuntimeEdit({
      input: updateInput({ runtime: "claude" }),
      managedAgent: agent(),
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [runtime({ availability: "cli_missing", command: null })],
    }),
    "Claude Code is not available. Install it before saving this linked agent.",
  );
});

test("validateLinkedAgentRuntimeEdit allows unchanged or unlinked runtime preferences", () => {
  assert.equal(
    validateLinkedAgentRuntimeEdit({
      input: updateInput({ runtime: "goose" }),
      managedAgent: agent(),
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [],
    }),
    null,
  );

  assert.equal(
    validateLinkedAgentRuntimeEdit({
      input: updateInput({ runtime: "claude" }),
      managedAgent: undefined,
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [],
    }),
    null,
  );
});

// Helpers to build a submit-options bundle with spy-able mutations. Mutations
// default to recording their calls so a test can assert spawn behavior.
function submitOptions(overrides = {}) {
  const calls = {
    createPersona: [],
    createManagedAgentForPersona: [],
    onDone: 0,
  };
  const createdPersona = persona({ id: "new-persona", displayName: "Fizz" });
  const options = {
    createManagedAgentForPersona: async (p) => {
      calls.createManagedAgentForPersona.push(p);
      return {
        agent: agent({ name: "Fizz", personaId: "new-persona" }),
        spawnError: null,
        profileSyncError: null,
      };
    },
    createPersona: async (input) => {
      calls.createPersona.push(input);
      return createdPersona;
    },
    input: createInput(),
    managedAgent: undefined,
    onDone: () => {
      calls.onDone += 1;
    },
    previousPersona: undefined,
    runtimes: [],
    templateOnly: undefined,
    updateManagedAgent: async () => {
      throw new Error("updateManagedAgent should not be called in create path");
    },
    updatePersona: async () => {
      throw new Error("updatePersona should not be called in create path");
    },
    ...overrides,
  };
  return { calls, options };
}

test("submitProfilePersonaDialog template-only creates the persona but spawns no agent", async () => {
  const { calls, options } = submitOptions({ templateOnly: true });

  await submitProfilePersonaDialog(options);

  assert.equal(calls.createPersona.length, 1, "persona template is created");
  assert.equal(
    calls.createManagedAgentForPersona.length,
    0,
    "no managed agent is spawned for a template-only save-as",
  );
  assert.equal(calls.onDone, 1, "dialog closes on success");
});

test("submitProfilePersonaDialog create path still spawns an agent when not template-only", async () => {
  const { calls, options } = submitOptions({ templateOnly: undefined });

  await submitProfilePersonaDialog(options);

  assert.equal(calls.createPersona.length, 1, "persona is created");
  assert.equal(
    calls.createManagedAgentForPersona.length,
    1,
    "legit create-and-spawn flow is unaffected",
  );
  assert.equal(calls.onDone, 1, "dialog closes on success");
});
