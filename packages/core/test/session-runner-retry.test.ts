import { describe, expect, test } from "bun:test"
import { AIError, ProviderInternalError, QuotaExceededError, RateLimitError, TransportError } from "@opencode/ai/effect"
import { Credential } from "@opencode/schema/credential"
import { ModelRetry } from "@opencode/core/model-retry"
import { Agent } from "@opencode/schema/agent"
import { Model } from "@opencode/schema/model"
import { Provider } from "@opencode/core/provider"
import { SessionSchema } from "@opencode/core/session/schema"
import { SessionRunnerRetry } from "@opencode/core/session/runner/retry"
import { toSessionError } from "@opencode/core/session/to-session-error"
import { Clock, Duration, Effect } from "effect"
import { TestClock } from "effect/testing"
import { it } from "./lib/effect"

const timeout = new AIError({
  reason: new TransportError({ message: "Timed out", transport: "http", operation: "request", code: "Timeout" }),
})
const internal = new AIError({ reason: new ProviderInternalError({ message: "internal" }) })
const dnsFailure = new AIError({
  reason: new TransportError({
    message: "getaddrinfo ENOTFOUND opencode.ai",
    transport: "http",
    operation: "request",
    code: "ENOTFOUND",
    url: "https://opencode.ai/zen/v1/responses",
  }),
})
const refused = new AIError({
  reason: new TransportError({
    message: "Unable to connect",
    transport: "http",
    operation: "request",
    code: "ECONNREFUSED",
  }),
})

const input = (cause: AIError) => ({
  cause,
  error: toSessionError(cause),
  agent: Agent.ID.make("build"),
  model: Model.Ref.make({ id: Model.ID.make("model"), providerID: Provider.ID.make("provider") }),
  hook: () => Effect.void,
  retry: SessionRunnerRetry.isRetryable(cause),
})

// Decisions for `count` consecutive failures of the same cause within one step.
const decisions = (cause: AIError, count: number) =>
  Effect.gen(function* () {
    const policy = yield* SessionRunnerRetry.policy(SessionSchema.ID.make("ses_retry"))
    const results: boolean[] = []
    for (let i = 0; i < count; i++) results.push((yield* policy(input(cause))).retry)
    return results
  })

describe("SessionRunnerRetry.policy", () => {
  it.effect("stops retrying transport timeouts after three attempts", () =>
    Effect.gen(function* () {
      expect(yield* decisions(timeout, 4)).toEqual([true, true, true, false])
    }),
  )

  it.effect("keeps the general allowance for other transient failures", () =>
    Effect.gen(function* () {
      expect(yield* decisions(internal, 11)).toEqual([...Array(10).fill(true), false])
    }),
  )

  it.effect("keeps retrying an unreachable host for an hour with gaps of at most 30 seconds", () =>
    Effect.gen(function* () {
      const policy = yield* SessionRunnerRetry.policy(SessionSchema.ID.make("ses_retry"))
      const started = yield* Clock.currentTimeMillis
      const delays: number[] = []
      while (true) {
        const decision = yield* policy(input(dnsFailure))
        if (!decision.retry) break
        delays.push(decision.delay)
        yield* TestClock.adjust(Duration.millis(decision.delay))
      }
      const waited = (yield* Clock.currentTimeMillis) - started
      expect(delays.length).toBeGreaterThan(100)
      expect(Math.max(...delays)).toBeLessThanOrEqual(30_000)
      expect(waited).toBeGreaterThanOrEqual(Duration.toMillis(ModelRetry.UNREACHABLE_WINDOW))
      expect(waited).toBeLessThanOrEqual(Duration.toMillis(ModelRetry.UNREACHABLE_WINDOW) + 30_000)
    }),
  )

  it.effect("keeps a refused connection on the general allowance", () =>
    Effect.gen(function* () {
      expect(ModelRetry.isUnreachable(refused)).toBe(false)
      expect(yield* decisions(refused, 11)).toEqual([...Array(10).fill(true), false])
    }),
  )

  it.effect("doesn't spend the general allowance on an unreachable host", () =>
    Effect.gen(function* () {
      const policy = yield* SessionRunnerRetry.policy(SessionSchema.ID.make("ses_retry"))
      for (let i = 0; i < 20; i++) expect((yield* policy(input(dnsFailure))).retry).toBe(true)
      const results: boolean[] = []
      for (let i = 0; i < 11; i++) results.push((yield* policy(input(internal))).retry)
      expect(results).toEqual([...Array(10).fill(true), false])
    }),
  )
})

describe("ModelRetry.retryOneShot", () => {
  it.effect("doesn't replay a call whose response was cut off by the network going down", () =>
    Effect.gen(function* () {
      let calls = 0
      const cutOff = new AIError({
        reason: new TransportError({
          message: "Network is down",
          transport: "http",
          operation: "read",
          code: "ENETDOWN",
        }),
      })
      yield* ModelRetry.retryOneShot(Effect.suspend(() => (calls++, Effect.fail(cutOff)))).pipe(Effect.flip)
      expect(calls).toBe(1)
    }),
  )
})

describe("toSessionError", () => {
  test("names the network when the provider's host can't be reached", () => {
    expect(toSessionError(dnsFailure)).toEqual({
      type: "provider.transport",
      message: "Can't reach opencode.ai. Check the network connection. (getaddrinfo ENOTFOUND opencode.ai)",
    })
    expect(toSessionError(refused)).toEqual({ type: "provider.transport", message: "Unable to connect" })
  })
})

describe("SessionRunnerRetry account rotation", () => {
  const rateLimit = new AIError({ reason: new RateLimitError({ message: "429", retryAfterMs: 60_000 }) })
  const quota = new AIError({ reason: new QuotaExceededError({ message: "insufficient_quota" }) })
  const ids = ["crd_a", "crd_b", "crd_c"].map((id) => Credential.ID.make(id))
  const accounts = () => {
    // Active first, as Integration lists connections.
    let order = [...ids]
    const activated: string[] = []
    return {
      activated,
      accounts: {
        saved: () => Effect.succeed(order),
        activate: (id: Credential.ID) =>
          Effect.sync(() => {
            activated.push(id)
            order = [id, ...order.filter((other) => other !== id)]
          }),
      },
    }
  }

  it.effect("switches to each other account once with no delay, then backs off", () =>
    Effect.gen(function* () {
      const state = accounts()
      const policy = yield* SessionRunnerRetry.policy(SessionSchema.ID.make("ses_rotate"), state.accounts)
      const first = yield* policy(input(rateLimit))
      const second = yield* policy(input(quota))
      expect(first).toEqual({ retry: true, attempt: 2, delay: 0 })
      expect(second).toEqual({ retry: true, attempt: 3, delay: 0 })
      expect(state.activated).toEqual(["crd_b", "crd_c"])
      // Every account tried: rate limits fall back to the provider's retry-after, quota stops.
      expect((yield* policy(input(rateLimit))).retry).toBe(true)
      expect(state.activated).toHaveLength(2)
      expect((yield* policy(input(quota))).retry).toBe(false)
    }),
  )

  it.effect("leaves other failures and single accounts to the normal policy", () =>
    Effect.gen(function* () {
      const state = accounts()
      const policy = yield* SessionRunnerRetry.policy(SessionSchema.ID.make("ses_rotate"), state.accounts)
      expect((yield* policy(input(internal))).retry).toBe(true)
      expect(state.activated).toEqual([])
      const single = yield* SessionRunnerRetry.policy(SessionSchema.ID.make("ses_rotate"), {
        saved: () => Effect.succeed([ids[0]!]),
        activate: () => Effect.die("unexpected"),
      })
      expect((yield* single(input(quota))).retry).toBe(false)
    }),
  )
})
