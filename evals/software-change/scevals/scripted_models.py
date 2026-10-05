"""No-network Inspect backends, reached through real Pi and the public bridge."""
import json
import re

from inspect_ai.model import (ChatCompletionChoice, ChatMessageAssistant, ModelAPI,
                              ModelOutput, ModelUsage, modelapi)
from inspect_ai.tool import ToolCall

from evalcore.target import OwnerApprovalRequired


@modelapi(name="owner-required")
class RequiredOwnerModel(ModelAPI):
    def __init__(self, *args, **kwargs):
        raise OwnerApprovalRequired("Owner-pending: replace every owner-required/ model ID with an owner-chosen Inspect model; paid-call approval is required before launch")

    async def generate(self, input, tools, tool_choice, config):
        raise AssertionError("owner gate must fail before generation")


@modelapi(name="sc-scripted")
class ScriptedModel(ModelAPI):
    async def generate(self, input, tools, tool_choice, config):
        if self.model_name == "simulator":
            question = input[-1].text.lower()
            answers = []
            ids = []
            for words, requirement, answer in (
                (("whitespace", "spaces"), "REQ-1", "Trim whitespace and collapse internal Unicode whitespace runs to one ASCII space."),
                (("unicode", "casing"), "REQ-2", "Use Unicode casefolding, so Straße and STRASSE group together."),
                (("empty", "blank"), "REQ-3", "Empty or whitespace-only labels use the key inbox."),
            ):
                if any(word in question for word in words):
                    ids.append(requirement)
                    answers.append(answer)
            content = json.dumps({"answer": " ".join(answers) or "Please ask about a specific label rule.",
                                  "revealed_requirement_ids": ids})
        elif self.model_name == "judge":
            content = '{"passed": true}'
        elif self.model_name == "driver" and any("Scripted driver: " in m.text for m in input):
            results = [m for m in input if m.role == "tool"]
            if not results:
                prompt = "\n".join(m.text for m in input)
                mode = re.search(r"Scripted driver: ([a-z-]+)", prompt).group(1)
                profile = re.search(r"Profile: ([a-z0-9-]+)", prompt).group(1)
                output = ModelOutput(model=self.model_name, choices=[ChatCompletionChoice(
                    message=ChatMessageAssistant(content="", tool_calls=[ToolCall(
                        id="scripted_run", function="bash", arguments={"command":
                            f"python3 /opt/sc-evals/scripted_driver.py {mode} {profile}", "timeout": 540})]),
                    stop_reason="tool_calls")])
                output.usage = ModelUsage(input_tokens=20, output_tokens=10, total_tokens=30)
                return output
            content = "routing-marker-driver"
        else:
            content = "routing-marker-" + self.model_name
        output = ModelOutput.from_content(model=self.model_name, content=content)
        output.usage = ModelUsage(input_tokens=20, output_tokens=10, total_tokens=30)
        return output
