import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveImportedPersonaAvatarUrl,
  toGooseAppAvatarRef,
} from "./gooseAppAvatarRefs.ts";

test("toGooseAppAvatarRef canonicalizes app-avatar refs", () => {
  assert.equal(
    toGooseAppAvatarRef("app-avatar:persona-19"),
    "app-avatar:persona-19",
  );
});

test("toGooseAppAvatarRef ignores filesystem-looking paths", () => {
  assert.equal(toGooseAppAvatarRef("./avatars/persona_2.png"), null);
});

test("resolveImportedPersonaAvatarUrl prefers app-avatar refs over data URLs", () => {
  assert.equal(
    resolveImportedPersonaAvatarUrl({
      avatarDataUrl: "https://example.com/avatar.png",
      avatarRef: "app-avatar:persona-1",
    }),
    "app-avatar:persona-1",
  );
});

test("resolveImportedPersonaAvatarUrl preserves ordinary image URLs", () => {
  assert.equal(
    resolveImportedPersonaAvatarUrl({
      avatarDataUrl: "https://example.com/avatar.png",
      avatarRef: null,
    }),
    "https://example.com/avatar.png",
  );
});

test("resolveImportedPersonaAvatarUrl does not rewrite remote image URLs", () => {
  assert.equal(
    resolveImportedPersonaAvatarUrl({
      avatarDataUrl: "https://cdn.example.com/avatars/persona_2.png",
      avatarRef: null,
    }),
    "https://cdn.example.com/avatars/persona_2.png",
  );
});

test("resolveImportedPersonaAvatarUrl preserves URL avatar refs", () => {
  assert.equal(
    resolveImportedPersonaAvatarUrl({
      avatarDataUrl: null,
      avatarRef: " https://example.com/persona-avatar.png ",
    }),
    "https://example.com/persona-avatar.png",
  );
});
