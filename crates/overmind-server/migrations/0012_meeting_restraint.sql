-- M13.5: restraint on meeting requests.
--
-- Agents must be autonomous but not free to flood the human. Three mechanisms:
-- hard caps on pending requests (enforced in meeting.rs), feedback so a
-- declined agent learns instead of re-asking forever, and the ability for a
-- room to conclude that no decision was needed at all.
--
-- `decline_note` carries the human's reason back INTO the agent's next prompt.
-- Without it a refusal is invisible to the agent that asked, and the same
-- request comes back on the next turn.
ALTER TABLE meetings ADD COLUMN decline_note TEXT;

-- `meetings.status` gains 'dropped': the room met (or started to) and concluded
-- there was nothing to decide. Distinct from 'decided' on purpose — a dropped
-- meeting must NOT be injected into anyone's work as a settled call.
