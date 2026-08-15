import {
  filterAdmittedMentionPubkeys,
  getAgentMentionAdmission,
  getMentionableAgentPubkeys,
  type AgentEligibilityScope,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import { evictUsersBatchEntries } from "@/features/profile/hooks";
import { getUsersBatch } from "@/shared/api/tauriProfiles";
import type {
  ManagedAgent,
  RelayAgent,
  UsersBatchResponse,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

type DirectoryResult<T> = {
  data: T | undefined;
  error: Error | null;
};

export async function revalidateAgentMentionPubkeys({
  pubkeys,
  agentPubkeys,
  currentPubkey,
  eligibilityScope,
  sharedChannelIds,
  ownerOnly,
  ownerPolicyError,
  refetchManagedAgents,
  refetchRelayAgents,
  refetchOwnerProfiles,
}: {
  pubkeys: readonly string[];
  agentPubkeys: ReadonlySet<string>;
  currentPubkey: string | null;
  eligibilityScope: AgentEligibilityScope;
  sharedChannelIds: ReadonlySet<string>;
  ownerOnly: boolean | undefined;
  ownerPolicyError: Error | null;
  refetchManagedAgents: () => Promise<DirectoryResult<ManagedAgent[]>>;
  refetchRelayAgents: () => Promise<DirectoryResult<RelayAgent[]>>;
  refetchOwnerProfiles: (pubkeys: string[]) => Promise<UsersBatchResponse>;
}) {
  const requestedAgentPubkeys = new Set(
    pubkeys.map(normalizePubkey).filter((pubkey) => agentPubkeys.has(pubkey)),
  );
  if (requestedAgentPubkeys.size === 0) {
    return [...pubkeys];
  }

  // Owner profiles are needed for every send (not just owner-only builds):
  // a relay-directory agent whose owner is the current user is a remotely
  // managed agent, and its mention must survive revalidation even when its
  // respondTo policy would otherwise exclude it. A failed fetch simply yields
  // an empty owned set below and the allow-list gate applies as before.
  const [managedResult, relayResult, ownerProfiles] = await Promise.all([
    refetchManagedAgents(),
    refetchRelayAgents(),
    refetchOwnerProfiles([...requestedAgentPubkeys]).catch(() => null),
  ]);
  if (
    managedResult.error !== null ||
    relayResult.error !== null ||
    managedResult.data === undefined ||
    relayResult.data === undefined ||
    ownerOnly === undefined ||
    ownerPolicyError !== null ||
    (ownerOnly && ownerProfiles === null)
  ) {
    return filterAdmittedMentionPubkeys(pubkeys, agentPubkeys, new Set());
  }

  const managedPubkeys = new Set(
    managedResult.data.map((agent) => normalizePubkey(agent.pubkey)),
  );
  // Same relaxation as the autocomplete surface: relay-directory agents whose
  // declared owner is the current user are remotely managed, so their selected
  // mention must not be stripped here. Profile-only identities absent from the
  // relay directory cannot appear in this set.
  const ownedRelayAgentPubkeys = new Set(
    (relayResult.data ?? []).flatMap((agent) => {
      const pubkey = normalizePubkey(agent.pubkey);
      const ownerPubkey = ownerProfiles?.profiles[pubkey]?.ownerPubkey;
      return ownerPubkey &&
        currentPubkey &&
        normalizePubkey(ownerPubkey) === normalizePubkey(currentPubkey)
        ? [pubkey]
        : [];
    }),
  );
  const mentionablePubkeys = getMentionableAgentPubkeys({
    currentPubkey,
    eligibilityScope,
    managedAgentPubkeys: managedPubkeys,
    relayAgents: relayResult.data,
    sharedChannelIds,
    ownedRelayAgentPubkeys,
  });
  const admittedPubkeys = new Set(
    [...agentPubkeys].filter(
      (pubkey) =>
        getAgentMentionAdmission({
          isAgent: true,
          isManagedAgent: managedPubkeys.has(pubkey),
          pubkey,
          ownerPubkey: ownerProfiles?.profiles[pubkey]?.ownerPubkey,
          currentPubkey,
          mentionableAgentPubkeys: mentionablePubkeys,
          directoryReady: true,
          ownerOnly,
        }) === "allow",
    ),
  );
  return filterAdmittedMentionPubkeys(pubkeys, agentPubkeys, admittedPubkeys);
}

export function useAgentMentionRevalidation({
  agentPubkeys,
  getSelectedAgentPubkeys,
  currentPubkey,
  eligibilityScope,
  sharedChannelIds,
  ownerOnly,
  ownerPolicyError,
  refetchManagedAgents,
  refetchRelayAgents,
}: {
  agentPubkeys: ReadonlySet<string>;
  getSelectedAgentPubkeys: () => ReadonlySet<string>;
  currentPubkey: string | null;
  eligibilityScope: AgentEligibilityScope;
  sharedChannelIds: ReadonlySet<string>;
  ownerOnly: boolean | undefined;
  ownerPolicyError: Error | null;
  refetchManagedAgents: () => Promise<DirectoryResult<ManagedAgent[]>>;
  refetchRelayAgents: () => Promise<DirectoryResult<RelayAgent[]>>;
}) {
  const queryClient = useQueryClient();
  const refetchOwnerProfiles = React.useCallback(
    async (pubkeys: string[]) => {
      evictUsersBatchEntries(queryClient, pubkeys);
      return getUsersBatch(pubkeys);
    },
    [queryClient],
  );
  return React.useCallback(
    (pubkeys: readonly string[]) =>
      revalidateAgentMentionPubkeys({
        pubkeys,
        agentPubkeys: new Set([...agentPubkeys, ...getSelectedAgentPubkeys()]),
        currentPubkey,
        eligibilityScope,
        sharedChannelIds,
        ownerOnly,
        ownerPolicyError,
        refetchManagedAgents,
        refetchRelayAgents,
        refetchOwnerProfiles,
      }),
    [
      agentPubkeys,
      currentPubkey,
      eligibilityScope,
      getSelectedAgentPubkeys,
      ownerOnly,
      ownerPolicyError,
      refetchManagedAgents,
      refetchOwnerProfiles,
      refetchRelayAgents,
      sharedChannelIds,
    ],
  );
}
