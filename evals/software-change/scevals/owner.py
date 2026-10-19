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


class SimulatedOwner:
    def __init__(self, case, model_id):
        self.case = case
        self.model_id = model_id
        self.answers = []  # Host-only question/answer text, never included in reports.

    async def ask_owner(self, question: str) -> str:
        if not isinstance(question, str) or not question.strip():
            raise ModelError("owner question must be nonempty text")
        model = get_model(self.model_id, role="simulator")
        response = await model.generate([
            ChatMessageSystem(content=OWNER_INSTRUCTION + "\nHidden brief:\n" + self.case.brief.read_text()),
            ChatMessageUser(content=question),
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
        self.answers.append({"question": question, "answer": answer["answer"],
                             "revealed_requirement_ids": sorted(set(ids))})
        return answer["answer"]

    def receipt(self):
        return {"model": self.model_id, "answer_count": len(self.answers),
                "answers": [{"index": i + 1, "revealed_requirement_ids": a["revealed_requirement_ids"]}
                            for i, a in enumerate(self.answers)],
                "revealed_requirement_ids": sorted({r for a in self.answers for r in a["revealed_requirement_ids"]})}
