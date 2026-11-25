# Capacity wait infrastructure

The PostgreSQL capacity-wait service is an opt-in planning primitive, not a
media start. It stores a five-minute deadline using the database clock and
processes live requests in insertion order. An unplaceable head blocks later
requests, including new immediate route-planning and standalone reservation
attempts. Immediate calls still return unavailable rather than joining the
queue. Worker heartbeats, worker changes, reservations, and queue changes emit
transactional PostgreSQL notifications; notifications are hints, not durable
events.

Database-backed API routers start a processor tied to the router's lifetime.
It subscribes to notifications, drains the queue, and uses a one-second
polling fallback, including while reconnecting. Database errors are retried
with bounded backoff. Multiple API instances are safe; a PostgreSQL advisory
lock serializes admission. The infrastructure module itself starts no task.

`enqueue(key, pipeline_id, route)` validates the selected revisions and
returns a `CapacityWaitRequest`. Its non-nil key shares the existing planning
idempotency namespace, with `allowWaiting: true` in its canonical request.
Reusing a key with different inputs, including switching waiting mode, is a
conflict. Replays return current wait status for 24 hours; terminal records
may be removed thereafter. `get(request_id)` returns `None` for an unknown or
purged request. `cancel(request_id)` returns the resulting status or `None`;
it only cancels a live wait. A concurrent successful plan may win the race,
in which case the caller must use the existing assignment cancellation flow.
Cancellation and expiry racing an in-flight attempt prevent its reservation
and assignment from committing.

States are `waiting`, `planned`, `unavailable`, and `cancelled`. For `planned`,
`result` contains the existing `PipelinePlanningDecision::Planned`. An
unavailable `reason` is `expired` or a planning error code (for example a
stale revision). The head is revalidated at every attempt. Expiry is terminal;
the same key never implicitly retries it. A deliberate new request needs a
new key and pipeline identity as appropriate. The processor also expires
requests behind an unplaceable head.

`POST /api/v1/routes/plan-assignment` opts in with `allowWaiting: true`;
`GET /api/v1/capacity-requests/{requestId}` reads status and
`POST /api/v1/capacity-requests/{requestId}/cancel` cancels a live wait. A
wait response includes `requestId` and `expiresAt`; its result uses the same
estimate and assignment fields as immediate planning. The UI stores a wait
attempt in tab session storage, polls its status, and can replay its original
idempotency key to recover an uncertain submission. Closing the tab discards
that local identity, not the durable server request. An explicit new request
is needed after expiry or cancellation.

Capacity units remain provisional and uncalibrated; no target hardware
validation is implied.
