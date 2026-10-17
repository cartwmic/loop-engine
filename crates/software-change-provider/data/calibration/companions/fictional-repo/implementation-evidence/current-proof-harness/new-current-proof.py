#!/usr/bin/env python3
"""Fresh current mechanics. Native campaign requires separate exact allocation."""
import sys
sys.dont_write_bytecode=True
import argparse,copy,json
from pathlib import Path
from types import SimpleNamespace
from proof_support import ProcessRunner,LIMITS,ENGINE_SHA,sha,inventory,digest,save,need
def sample(schema):
    if schema["type"]=="object":
        return {k:sample(schema["properties"][k]) for k in schema.get("required",[])}
    if schema["type"]=="array":
        return [sample(schema["items"]) for _ in range(max(1,schema.get("minItems",0)))]
    return schema.get("enum",["SCRIPTED mechanics not semantic judgment"])[0]
write=save
def main():
    need(__debug__, "proof assertions disabled")
    p=argparse.ArgumentParser(description=__doc__)
    for name in ["source","engine","output-root"]: p.add_argument("--"+name,type=Path,required=True)
    for name in ["python","cargo","rustc"]: p.add_argument("--"+name,type=Path)
    for name in ["jobs","threads"]: p.add_argument("--"+name,type=int,default=2,choices=[2])
    p.add_argument("--grant-file",type=Path)
    p.add_argument("--scripted-backend",type=Path)
    p.add_argument("--plan",action="store_true")
    args=p.parse_args()
    if args.plan:
        print(json.dumps({"status":"PREPARATION_ONLY","limits":LIMITS,"source_sha256":digest(inventory(args.source))}))
        return
    tools={name:str(getattr(args,name)) for name in ["python","cargo","rustc"] if getattr(args,name) is not None}
    runner=ProcessRunner(args.source,args.engine,args.output_root,args.scripted_backend,args.grant_file,tools)
    cargo=tools.get("cargo","cargo")  # PATH lookup only in explicit scripted test mode.
    fixture=runner.source; engine=runner.engine; output=runner.output
    receipt={"generation":"new-current-01","backend":runner.mode,"status":"running","journeys":[],
             "source_before":runner.before,"engine_sha256":sha(engine),"mechanical_only":True,"semantic_review":False}
    identity=[]; linked=[]
    write(output/"runtime-current.json",receipt)
    def call(argv,packet=None,expected=0,timeout=30):
        row=runner.call(argv,packet,expected,timeout)
        return SimpleNamespace(stdout=row["stdout"],stderr=row["stderr"],returncode=expected,index=row["index"])
    try:
        call([cargo,"build","--manifest-path",fixture/"Cargo.toml","-p","software-change-provider","--locked","--offline","--jobs","2"],timeout=1200)
        meta=json.loads(call([cargo,"metadata","--manifest-path",fixture/"Cargo.toml","--format-version","1","--no-deps","--offline"]).stdout)
        target=Path(meta["target_directory"]).resolve()
        need(target==output/"build","unexpected target directory")
        provider=target/"debug/software-change"; need(provider.is_file(),"provider missing")
        receipt.update(provider_path=str(provider),provider_sha256=sha(provider))
        describe=call([provider],{"operation":"describe"})
        need(describe.stdout.encode()==(fixture/"provider/tests/snapshots/describe.json").read_bytes(),"describe differs")
        need(call([provider],{"operation":"describe","initial_input":{"opaque":"not policies"}}).stdout==describe.stdout,"opaque describe differs")
        for packet in [{"operation":"describe","unexpected":True},{"operation":"unknown"}]:
            need(not call([provider],packet,expected=2).stdout,"bad envelope emits JSON")
        call([provider,"data-dump",output/"references"])
        need(inventory(output/"references")==inventory(fixture/"provider/data"),"reference dump differs")
        receipt["protocol"]=["describe-exact","opaque-describe-exact","unexpected-envelope-refused","unknown-operation-refused","data-dump-exact"]
        config=output/"providers.toml"
        config.write_text("[providers.software-change]\ncommand = "+json.dumps(str(provider))+"\nargs = []\n")
        base=[engine,"--json","--database",output/"run.sqlite","--config",config]
        def cli(*argv,status="completed"):
            before=cli("show",argv[1],"--view","full")["result"] if argv[0]=="event" else None
            result=call(base+list(argv),expected={"completed":0,"rejected":10,"error":20}[status])
            value=json.loads(result.stdout)
            need(value["status"]==status,"wrong CLI status")
            if before is not None:
                after=cli("show",argv[1],"--view","full")["result"]
                need(before["initial_input"]==after["initial_input"],"event changed frozen input")
                if status!="completed":
                    need(before["current_state"]==after["current_state"] and before["context"]==after["context"],"refused event mutated state/context")
            value["_command_index"]=result.index
            return value
        source_data=fixture/"provider/data"
        for profile_name in ["minimal", "standard", "high-rigor"]:
            profile_path = source_data / ("configs/" + profile_name + ".json")
            submitted = json.loads(profile_path.read_text())
            rid = "new-current-01-" + profile_name
            cli("start", "--id", rid, "software-change", "@" + str(profile_path),
                "NEW fictional deterministic proof; not historic observation")
            full = cli("show", rid, "--view", "full")["result"]
            frozen = full["initial_input"]
            assert {k: v for k, v in frozen.items() if k != "artifact_root"} == submitted
            assert set(frozen) == set(submitted) | {"artifact_root"}
            assert "templates" not in frozen and "reviewer_protocol" not in frozen
            root = Path(frozen["artifact_root"]).resolve()
            assert root.is_relative_to(output)
            root.mkdir(parents=True, exist_ok=True)
            documents = {}
            for name, schema in submitted["artifact_schemas"].items():
                doc = sample(schema)
                doc.update(revision="NEW-r1", author={"name": "fixture-subject-author", "kind": "human"})
                documents[name] = doc
            for link in submitted.get("revision_links", []):
                documents[link["from"]][link["field"]] = documents[link["to"]]["revision"]
            # Real missing/unparseable/exhaustive structural negative, not context records
            # pretending to materialize authored files.
            cli("event", rid, "intent-ready", status="rejected")
            (root / "intent.json").write_text("not JSON")
            cli("show", rid, "--view", "action")
            cli("event", rid, "intent-ready", status="rejected")
            malformed = copy.deepcopy(documents["intent.json"])
            del malformed["revision"]
            malformed["outcome"] = ""
            malformed["NEW-unexpected"] = "defect"
            write(root / "intent.json", malformed)
            cli("show", rid, "--view", "action")
            denied = cli("event", rid, "intent-ready", status="rejected")
            diagnostics = denied["details"]
            actual = {(d["path"], d["rule"]) for d in diagnostics}
            assert any(d["path"] == "" and d["rule"] == "required"
                       and d["subject"] == "intent.json"
                       and d["message"] == "required property `revision` is missing"
                       for d in diagnostics), denied
            assert ("/outcome", "minLength") in actual, denied
            assert ("/NEW-unexpected", "additionalProperties") in actual, denied
            for name, doc in documents.items():
                write(root / name, doc)
            transition_count = 0
            serial = 0
            negatives = []

            def append(data, kind="review-evidence"):
                nonlocal serial
                serial += 1
                cli("show", rid, "--view", "full")
                cli("append", "--record-id", "proof-" + str(serial), rid, kind, json.dumps(data))

            while True:
                full = cli("show", rid, "--view", "full")["result"]
                assert full["initial_input"] == frozen
                state = full["current_state"]
                if state == "end":
                    # Installed0.25.0 lifecycle is final; completed is the separate
                    # ordinary completion-mode projection, not lifecycle vocabulary.
                    assert full["lifecycle"] == "final"
                    assert full["requestable_events"] == []
                    assert full["completion_mode"] == "completed"
                    assert full["has_overrides"] is False and full["override_count"] == 0
                    assert any(row["id"] == state and row["final"] is True
                               for row in full["workflow_graph"]["states"])
                    history = cli("history", rid)["result"]
                    assert isinstance(history, list) and history
                    sequences = [row["sequence"] for row in history]
                    assert sequences == sorted(set(sequences))
                    by_sequence = {row["sequence"]: row for row in history}
                    denied_evaluations = [row for row in full["evaluation_history"]
                                          if row["result"]["result"] == "deny"]
                    assert {"software-change-schema-deny", "software-change-evidence-incomplete"} <= {
                        row["result"]["feedback"]["code"] for row in denied_evaluations}
                    for evaluation in denied_evaluations:
                        action = by_sequence[evaluation["sequence"]]["action"]
                        assert action["kind"] == "transition"
                        assert action["transition"] == evaluation["transition"]
                        assert action["outcome"]["outcome"] == "denied"
                        assert action["outcome"]["feedback"] == evaluation["result"]["feedback"]
                    assert any(row["action"]["kind"] == "transition"
                               and row["action"]["transition"]["target"] == "end"
                               and row["action"]["outcome"]["outcome"] == "committed"
                               for row in history)
                    receipt["journeys"].append({"profile": profile_name, "run_id": rid,
                        "state": state, "final_show": full, "final_show_index": runner.commands[-2]["index"],
                        "history_index": runner.commands[-1]["index"], "transitions": transition_count,
                        "negative_cases": negatives, "frozen_input": frozen,
                        "artifact_root": str(root), "history": history})
                    break
                edge = next(t for t in full["workflow_graph"]["transitions"]
                            if t["source"] == state and t["kind"] == "checked")
                axes = submitted["review_policies"].get(state, [])
                if axes:
                    before_state = full["current_state"]
                    cli("event", rid, edge["event"], status="rejected")
                    assert cli("show", rid, "--view", "full")["result"]["current_state"] == before_state
                    subject = ("validation-report.json" if state.startswith("validation") else
                               "implementation-report.json" if state.startswith("implementation") else
                               state.split("-")[0] + ".json")
                    good = [{"gate": state, "policy_id": axis["id"], "subject": subject,
                             "subject_revision": documents[subject]["revision"],
                             "config_version": submitted["config_version"],
                             "author": {"name": "reviewer-" + str(i), "kind": "script"},
                             "result": "pass", "findings": ""}
                            for axis in axes for i in range(axis.get("required_authors", 1))]
                    for record in good[:-1]:
                        append(record)
                    missing = good[-1]
                    # Decisive bad contribution; each repair adds only the missing judgment.
                    for defect in ["stale-revision", "stale-config", "subject-author"]:
                        bad = copy.deepcopy(missing)
                        if defect == "stale-revision":
                            bad["subject_revision"] = "OLD-r0"
                        elif defect == "stale-config":
                            bad["config_version"] = "OLD-profile"
                        else:
                            bad["author"] = documents[subject]["author"]
                        append(bad)
                        cli("show", rid, "--view", "full")
                        cli("event", rid, edge["event"], status="rejected")
                        negatives.append({"gate": state, "case": defect})
                    if axes[-1].get("required_authors", 1) > 1:
                        bad = copy.deepcopy(missing)
                        bad["author"] = good[-2]["author"]
                        append(bad)
                        cli("show", rid, "--view", "full")
                        cli("event", rid, edge["event"], status="rejected")
                        negatives.append({"gate": state, "case": "duplicate-author"})
                    if state == "intent-review":
                        failed = copy.deepcopy(missing)
                        failed.update(result="fail", findings="SCRIPTED standing failure witness")
                        append(failed)
                        other = copy.deepcopy(missing)
                        other["author"]["name"] = "additional-distinct-reviewer"
                        append(other)
                        cli("show", rid, "--view", "full")
                        denied = cli("event", rid, edge["event"], status="rejected")
                        assert any(d.get("standing_fails", 0) > 0 for d in denied["details"])
                        negatives.append({"gate": state, "case": "standing-fail-not-outvoted"})
                    append(missing)
                    cli("show", rid, "--view", "full")
                cli("event", rid, edge["event"])
                transition_count += 1
                assert transition_count <= 30, "unexpected topology"
        names=["valid-control","stale-with-sufficient","stale-noncounting","stale-malformed-order-unchanged","standing-fail-and-own-restoration"]
        names += ["identity-"+field+"-"+shape for field in ["subject_revision","config_version"] for shape in ["missing","non-string","blank"]]
        counter=0
        def add(rid,row):
            nonlocal counter
            counter+=1
            cli("append","--record-id","identity-"+str(counter),rid,"review-evidence",json.dumps(row))
        for name in names:
            rid="new-current-control-"+name
            profile=json.loads((source_data/"configs/high-rigor.json").read_bytes())
            cli("start","--id",rid,"software-change","@"+str(source_data/"configs/high-rigor.json"),"NEW CURRENT identity control")
            full=cli("show",rid,"--view","full")["result"]
            root=Path(full["initial_input"]["artifact_root"]).resolve()
            need(root.is_relative_to(output),"identity artifact root escape"); root.mkdir(parents=True,exist_ok=True)
            docs={}
            for fn,schema in profile["artifact_schemas"].items():
                docs[fn]=sample(schema); docs[fn].update(revision="NEW-CURRENT-r1",author={"name":"subject","kind":"human"})
            for link in profile.get("revision_links",[]): docs[link["from"]][link["field"]]=docs[link["to"]]["revision"]
            for fn,doc in docs.items(): write(root/fn,doc)
            cli("event",rid,"intent-ready")
            full=cli("show",rid,"--view","full")["result"]; state=full["current_state"]
            need(state=="intent-review","wrong identity gate")
            edge=next(e for e in full["workflow_graph"]["transitions"] if e["source"]==state and e["kind"]=="checked")
            good=[{"gate":state,"policy_id":axis["id"],"subject":"intent.json","subject_revision":docs["intent.json"]["revision"],
                   "config_version":profile["config_version"],"author":{"name":"reader-"+str(i),"kind":"script"},"result":"pass","findings":""}
                  for axis in profile["review_policies"][state] for i in range(axis.get("required_authors",1))]
            sufficient=name in ["valid-control","stale-with-sufficient","stale-malformed-order-unchanged"]
            for row in good if sufficient else good[:-1]: add(rid,row)
            missing=good[-1]; bad=copy.deepcopy(missing); expected="completed"; restore=False
            if name.startswith("identity-"):
                rest=name.removeprefix("identity-")
                field=next(f for f in ["subject_revision","config_version"] if rest.startswith(f+"-"))
                shape=rest.removeprefix(field+"-")
                if shape=="missing": del bad[field]
                else: bad[field]=42 if shape=="non-string" else " "
                add(rid,bad); expected="rejected"; restore=True
            elif name.startswith("stale-"):
                bad["config_version"]="STALE-profile"
                if name=="stale-malformed-order-unchanged": bad["author"]=None; bad["result"]="invalid"; bad["findings"]=None
                add(rid,bad)
                if name=="stale-noncounting": expected="rejected"; restore=True
            elif name=="standing-fail-and-own-restoration":
                bad.update(result="fail",findings="SCRIPTED standing failure"); add(rid,bad)
                other=copy.deepcopy(missing); other["author"]["name"]="other-cannot-outvote"; add(rid,other)
                expected="rejected"; restore=True
            before=cli("show",rid,"--view","full")["result"]
            observation=cli("event",rid,edge["event"],status=expected)
            after=cli("show",rid,"--view","full")["result"]
            restored=None
            if restore:
                details=observation["details"]
                need(any(d.get("standing_fails",0)>0 for d in details) if name=="standing-fail-and-own-restoration" else
                     any(d.get("rule") in ["malformed-current-evidence","stale-config","axis-incomplete-or-failing"] for d in details),
                     "wrong identity refusal cause")
                add(rid,missing); restored=cli("event",rid,edge["event"])
            identity.append({"case":name,"expected":expected,"before":before,"event":observation,"after":after,"restoration":restored})
        local=output/"linked-artifacts"; external=output/"linked-outside"; local.mkdir(); external.mkdir()
        actor={"name":"fixture","kind":"script"}
        base_schema={"type":"object","additionalProperties":False,"required":["revision","author"],"properties":{
          "revision":{"type":"string","minLength":1},"author":{"type":"object","additionalProperties":False,"required":["name","kind"],
          "properties":{"name":{"type":"string","minLength":1},"kind":{"type":"string","enum":["human","agent","script"]}}}}}
        design_schema=copy.deepcopy(base_schema); design_schema["properties"]["intent_revision"]={"type":"string"}
        workflow=json.loads(describe.stdout)
        edge=next(e for e in workflow["transitions"] if e["source"]=="design" and e["event"]=="design-ready")
        packet={"operation":"evaluate","workflow":workflow,"context":[],"initial_input":{"config_version":"fixture-7","artifact_root":str(local),
                "artifact_schemas":{"intent.json":base_schema,"design.json":design_schema},"review_policies":{},
                "revision_links":[{"from":"design.json","field":"intent_revision","to":"intent.json"}],"extra":{}},"transition":edge,"prior_evaluations":[]}
        write(local/"design.json",{"revision":"r1","author":actor,"intent_revision":"r1"})
        for name in ["missing","mismatch","escape","valid"]:
            parent=local/"intent.json"
            if name=="mismatch": write(parent,{"revision":"different"})
            if name=="escape":
                parent.unlink(); write(external/"intent.json",{"revision":"r1"}); parent.symlink_to(external/"intent.json")
            if name=="valid":
                parent.unlink(); write(parent,{"revision":"r1"})
            result=call([provider],packet,expected=1 if name=="escape" else 0)
            if name=="escape": need(not result.stdout and "artifact intent.json escapes configured root" in result.stderr,"containment taxonomy")
            else:
                value=json.loads(result.stdout)
                if name=="valid": need(value=={"result":"allow"},"valid parent not allowed")
                else: need(value["result"]=="deny" and any(d["rule"]=="revision-link" and d["subject"]=="design.json" and d["target"]=="intent.json"
                             for d in value["feedback"]["details"]),"missing/mismatch taxonomy")
            linked.append({"case":name,"command_index":result.index})
        receipt["source_after"]=inventory(fixture)
        need(receipt["source_before"]==receipt["source_after"],"source changed")
        need(sha(engine)==receipt["engine_sha256"] and sha(provider)==receipt["provider_sha256"],"executable changed")
        accounting=runner.finish(True)
        if accounting["success"] is not True or accounting.get("limit_error") is not None:
            write(output/"failed-final-accounting.json", accounting)
            raise RuntimeError("unsuccessful final accounting: "+repr(accounting.get("limit_error")))
        need(accounting["all_children_waited"] and not accounting["residual_groups"],"unreconciled children")
        receipt.update(status="scripted-complete" if runner.mode=="scripted" else "passed",accounting_sha256=sha(output/"final-accounting.json"))
        write(output/"runtime-current.json",receipt)
        write(output/"identity-current.json",{"generation":"new-current-01","backend":runner.mode,"cases":identity})
        write(output/"linked-parent-current.json",{"generation":"new-current-01","backend":runner.mode,"cases":linked})
        write(output/"current-binding.json",{"generation":"new-current-01","backend":runner.mode,"source_sha256":digest(runner.before),
              "runtime_sha256":sha(output/"runtime-current.json"),"identity_sha256":sha(output/"identity-current.json"),
              "linked_sha256":sha(output/"linked-parent-current.json"),"accounting_sha256":sha(output/"final-accounting.json"),
              "engine_sha256":receipt["engine_sha256"],"provider_sha256":receipt["provider_sha256"]})
    except BaseException as error:
        receipt.update(status="failed",failure=repr(error),source_after=inventory(fixture))
        runner.finish(False); write(output/"runtime-current.json",receipt); raise
    print(json.dumps({"status":receipt["status"],"backend":runner.mode,"output":str(output),"processes":len(runner.commands),"native_proof":runner.mode=="native"}))
if __name__=="__main__": main()
