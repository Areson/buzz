import type { ParsedPersonaPreview } from "@/shared/api/tauriPersonas";
import type { CreatePersonaInput } from "@/shared/api/types";
import { resolveImportedPersonaAvatarUrl } from "@/shared/avatars/gooseAppAvatarRefs";

export function buildBatchImportPersonaInput(
  persona: ParsedPersonaPreview,
): CreatePersonaInput {
  const avatarUrl = resolveImportedPersonaAvatarUrl(persona);

  return {
    displayName: persona.displayName,
    avatarUrl: avatarUrl ?? undefined,
    systemPrompt: persona.systemPrompt,
    runtime: persona.runtime ?? undefined,
    model: persona.model ?? undefined,
    provider: persona.provider ?? undefined,
    namePool: persona.namePool.length > 0 ? persona.namePool : undefined,
  };
}
