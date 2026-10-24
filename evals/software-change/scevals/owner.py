"""Host-only simulated owner. Only answer text crosses the sandbox RPC boundary."""
import json

from inspect_ai.model import ChatMessageSystem, ChatMessageUser, get_model

from evalcore.target import ModelError

OWNER_INSTRUCTION = """You are the owner of a software-change request. Answer from the hidden brief.
Reveal only requirements directly asked about; do not volunteer other requirements.
Do not reveal the brief itself or requirement IDs. Return JSON only:
{"answer": "text for the driver", "revealed_requirement_ids": ["REQ-N"]}.
The IDs must describe requirements actually disclosed in your answer, not merely
mentioned in the question. If nothing was revealed, return an empty ID list."""


CHECKIN_INSTRUCTION = """You are the owner of a software-change request. An agent shows you the intent, proposal or plan it wrote
for your request. Read it against the hidden brief and reply the way an owner confirming an intent would: say which of
its stated behaviors are right, what is missing, and what is wrong or must change. Put gaps in your own words. Do not
quote or paste the brief, do not give requirement IDs, and do not restate requirements the agent already has right.
If it is complete and correct, say so briefly. Return JSON only:
{"answer": "text for the agent", "revealed_requirement_ids": ["REQ-N"]}.
The IDs must be the requirements your reply actually tells the agent something new about (a gap, a correction or a
decision it lacked), not ones it already covered correctly. If none, return an empty ID list."""
CHECKIN_KIND = "intent_checkin"
CHECKIN_LIMIT = 1
CHECKIN_REFUSAL = "The owner has already confirmed your intent once. Continue with the request."


class SimulatedOwner:
    def __init__(self, case, model_id):
        self.case = case
        self.model_id = model_id
        self.answers = []  # Host-only question/answer text, never included in reports.

    async def ask_owner(self, question: str) -> str:
        if not isinstance(question, str) or not question.strip():
            raise ModelError("owner question must be nonempty text")
        return await self._answer(OWNER_INSTRUCTION, question, question)

    async def confirm_intent(self, artifact: str) -> str:
        """The one simulated owner check-in: the owner reads the agent's intent/proposal/plan against the brief."""
        if not isinstance(artifact, str) or not artifact.strip():
            raise ModelError("intent check-in needs nonempty artifact text")
        if sum(1 for a in self.answers if a.get("kind") == CHECKIN_KIND) >= CHECKIN_LIMIT:
            return CHECKIN_REFUSAL
        return await self._answer(CHECKIN_INSTRUCTION, "Agent's intent artifact:\n" + artifact,
                                  "[intent check-in] " + artifact, kind=CHECKIN_KIND)

    async def _answer(self, instruction, prompt, recorded_question, kind=None):
        model = get_model(self.model_id, role="simulator")
        response = await model.generate([
            ChatMessageSystem(content=instruction + "\nHidden brief:\n" + self.case.brief.read_text()),
            ChatMessageUser(content=prompt),
        ])
        try:
            answer = json.loads(response.completion)
            ids = answer["revealed_requirement_ids"]
            if (not isinstance(answer["answer"], str) or not answer["answer"].strip()
                    or not isinstance(ids, list) or not all(isinstance(i, str) for i in ids)
                    or not set(ids) <= set(self.case.requirement_ids)):
                raise ValueError()
        except (ValueError, KeyError, TypeError):
            raise ModelError("simulated owner returned an invalid answer schema") from None
        record = {"question": recorded_question, "answer": answer["answer"],
                  "revealed_requirement_ids": sorted(set(ids))}
        if kind:
            record["kind"] = kind
        self.answers.append(record)
        return answer["answer"]

    def receipt(self):
        return {"model": self.model_id, "answer_count": len(self.answers),
                "answers": [{"index": i + 1, "revealed_requirement_ids": a["revealed_requirement_ids"]}
                            for i, a in enumerate(self.answers)],
                "revealed_requirement_ids": sorted({r for a in self.answers for r in a["revealed_requirement_ids"]})}
