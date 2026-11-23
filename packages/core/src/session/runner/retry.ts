export * as SessionRunnerRetry from "./retry.js"

import { AIError, isRetryable } from "@opencode/ai/effect"
import { Agent } from "@opencode/schema/agent"
import { Credential } from "@opencode/schema/credential"
import { Model } from "@opencode/schema/model"
import { SessionError } from "@opencode/schema/session-error"
import { Clock, Duration, Effect } from "effect"
import { Bus } from "../../bus.js"
import { ModelRetry } from "../../model-retry.js"
import type { PluginHooks } from "../../plugin/hooks.js"
import { SessionEvent } from "../event.js"
import { SessionMessage } from "../message.js"
import { SessionSchema } from "../schema.js"

export { isRetryable }

interface Input {
  readonly cause: AIError
  readonly error: SessionError.Error
  readonly agent: Agent.ID
  readonly model: Model.Ref
  readonly hook: (event: PluginHooks.Domains["session"]["retry"]) => Effect.Effect<void>
  readonly retry: boolean
}

export interface Decision {
  readonly retry: true
  readonly attempt: number
  readonly delay: number
}

/** Failures that mean the current account is out of capacity, not that the request is wrong. */
export const isLimit = (cause: AIError) => cause.reason._tag === "RateLimit" || cause.reason._tag === "QuotaExceeded"

/** The saved accounts of a model's provider, so a limited one can hand over to the next. */
export interface Accounts {
  /** Saved credential IDs for the provider's integration, the active one first. */
  readonly saved: (model: Model.Ref) => Effect.Effect<ReadonlyArray<Credential.ID>>
  readonly activate: (id: Credential.ID) => Effect.Effect<void>
}

/**
 * On a rate limit or quota failure, activates the provider's next saved account not yet tried by this policy and
 * reports whether it did. Each account is tried once per policy, so a step where every account is limited falls back
 * to the normal backoff instead of cycling.
 */
const rotation = (accounts: Accounts) => {
  const tried = new Set<Credential.ID>()
  return (input: Input) =>
    Effect.gen(function* () {
      if (!isLimit(input.cause)) return false
      const saved = yield* accounts.saved(input.model)
      // The active account is the one that just hit the limit.
      if (saved[0]) tried.add(saved[0])
      const next = saved.find((id) => !tried.has(id))
      if (!next) return false
      tried.add(next)
      yield* accounts.activate(next)
      yield* Effect.logInfo("switched account after provider limit", {
        providerID: input.model.providerID,
        credentialID: next,
        reason: input.cause.reason._tag,
      })
      return true
    })
}

export const policy = (sessionID: SessionSchema.ID, accounts?: Accounts) =>
  Effect.gen(function* () {
    const decide = yield* ModelRetry.policy
    const rotate = accounts ? rotation(accounts) : undefined
    let attempt = 1
    return (input: Input) =>
      Effect.gen(function* () {
        // A fresh account has its own limits, so retry on it at once without spending the backoff allowance.
        const next = rotate && (yield* rotate(input)) ? { retry: true, delay: 0 } : yield* decide(input)
        if (!next) return { retry: false as const }
        attempt++
        const event: PluginHooks.Domains["session"]["retry"] = {
          sessionID,
          agent: input.agent,
          model: input.model,
          error: input.error,
          attempt,
          decision: next.retry ? { retry: true, delay: next.delay } : { retry: false },
        }
        yield* input.hook(event)
        if (!event.decision.retry) return event.decision
        const normalized =
          Number.isFinite(event.decision.delay) && event.decision.delay >= 0
            ? Math.ceil(event.decision.delay)
            : next.delay
        return { retry: true as const, attempt, delay: normalized }
      })
  })

export const make = (bus: Bus.Interface, sessionID: SessionSchema.ID, accounts?: Accounts) =>
  Effect.gen(function* () {
    const decide = yield* policy(sessionID, accounts)
    const wait = (input: {
      readonly decision: Decision
      readonly assistantMessageID: SessionMessage.ID
      readonly error: SessionError.Error
    }) =>
      Effect.gen(function* () {
        const scheduled = yield* Clock.currentTimeMillis
        yield* bus.publish(SessionEvent.RetryScheduled, {
          sessionID,
          assistantMessageID: input.assistantMessageID,
          attempt: input.decision.attempt,
          at: scheduled + input.decision.delay,
          error: input.error,
        })
        const remaining = Math.max(0, scheduled + input.decision.delay - (yield* Clock.currentTimeMillis))
        yield* Effect.sleep(Duration.millis(remaining))
      })
    return { decide, wait }
  })
