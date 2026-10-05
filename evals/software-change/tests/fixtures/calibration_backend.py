"""Scripted Inspect backend, loaded by actual evalctl; no API/network calls."""
import json

from inspect_ai.model import ModelAPI, ModelOutput, ModelUsage, modelapi


@modelapi(name="calibration-fixture")
class CalibrationBackend(ModelAPI):
    async def generate(self, input, tools, tool_choice, config):
        payload = json.loads(input[-1].content)
        assert "owner_brief" in payload and "intent" in payload
        if self.model_name == "must-not-call":
            raise AssertionError("unconfirmed labels triggered a judge call")
        if self.model_name == "invalid":
            content = '{"passed": "true"}'
        else:
            # Four fixed known judgments: true positive, false negative,
            # false positive, true negative. The caller does not supply verdicts.
            content = json.dumps({"passed": payload["intent"]["fixture_id"] in ("good-pass", "deficient-pass")})
        output = ModelOutput.from_content(model=self.model_name, content=content)
        output.usage = ModelUsage(input_tokens=10, output_tokens=2, total_tokens=12)
        return output


def register_subcommands(subparsers):
    pass
