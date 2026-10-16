"""NEW toy workspace derivative of current outcome-spine configure ONLY.
Test-local source and catalogs only.
"""
import json, sys, copy
from pathlib import Path
from capture_support import require

def configure(profile, repository, artifacts, log, args):
    def process(argv):
        return log.execute(argv, cwd=repository)
    profile['extra']['bookends'] = {'enabled':True}
    absent = process([args.provider,'bookends-preview','--working-directory',repository])
    assert absent['missing_prerequisites'] and not absent['gate_executed'] and not absent['tests_executed'], absent
    (repository / 'docs').mkdir()
    (repository / 'scripts').mkdir()
    (repository / '.github/workflows').mkdir(parents=True)
    (repository / 'docs/PRD.md').write_text('### LE-1: Public helper outcome\n- Status: live\n- Coverage: e2e/journey\n\nAn external caller imports helper and observes helper(7) return 7. The applicable boundary is in [contract](contract.md).\n')
    (repository / 'docs/contract.md').write_text('# Public contract\n\nThe supported helper import returns the caller-supplied value; engine workflow status is not this outcome.\n')
    (repository / 'bookends.toml').write_text('prd = "docs/PRD.md"\n[classes.e2e_journey]\npathspecs = ["scripts/helper-journey.py"]\nrequired_ci_jobs = ["journey"]\n')
    (repository / '.github/workflows/ci.yml').write_text('jobs:\n  journey:\n    steps:\n      - run: python3 scripts/helper-journey.py\n')
    (repository / 'scripts/helper-journey.py').write_text('# bookends:LE-1\nimport sys\nfrom pathlib import Path\nsys.path.insert(0,str(Path(__file__).resolve().parents[1]))\nfrom helper import helper\nassert helper(7) == 7, "public helper must return the supplied value"\n')
    require(log.execute(['git','add','docs','scripts','.github','bookends.toml'], cwd=repository, raw=True).returncode == 0, 'Toy fixture add failed')
    require(log.execute(['git','commit','-qm','NEW toy Bookends fixture'], cwd=repository, raw=True).returncode == 0, 'Toy fixture commit failed')
    preview = process([args.provider,'bookends-preview','--working-directory',repository])
    assert not preview['gate_executed'] and not preview['tests_executed'] and preview['evaluation_cwd'] == str(repository)
    wording = preview['prd']['live_requirements'][0]
    assert wording['id'] == 'LE-1' and 'helper(7)' in wording['normative_text'] and wording['explicit_references'], wording
    surface = preview['proof_classes'][0]
    assert 'scripts/helper-journey.py' in surface['eligible_public_locations'], surface
    intent_path = artifacts / 'intent.json'
    intent = json.loads(intent_path.read_text())
    intent.update(problem='The public helper must return its supplied value.',outcome='An external importing caller observes helper(7) return 7.',
                  constraints=['Use the existing public helper import and actual external proof output.'],
                  non_goals=['Changing engine semantics or claiming semantic review quality.'])
    intent['operating_context'].update(operators=['trusted local fixture caller'],environment=['isolated repository','public Python import'],
                  threat_boundary={'in_scope':['public helper return value'],'excluded':['production delivery','model quality']},accepted_risks=[])
    intent['acceptance'] = [{'id':'AC-1','statement':'The public helper journey imports helper and asserts helper(7) == 7.',
                             'prd_traceability':{'type':'linked-live','live_ids':['LE-1']}}]
    intent['operating_context']['outside_obligations'] = [{'source':'docs/contract.md','obligation':'The supported import returns the caller-supplied value.'}]
    intent_path.write_text(json.dumps(intent))
    design_path = artifacts / 'design.json'
    design = json.loads(design_path.read_text())
    design['approach'] = 'Keep the public helper import returning the supplied argument, and use the public script to observe it.'
    design['elements'] = [{'name':'public helper import','responsibility':'Return the caller-supplied value.'},
                          {'name':'public journey','responsibility':'Import helper and distinguish an incorrect return value.'}]
    design['decisions'] = [{'choice':'Assert the actual return through the public import.',
                            'rationale':'Workflow status or a citation token cannot establish the return value.',
                            'rejected':['Checking only command exit or artifact presence.']}]
    design['risks'] = [{'risk':'A token-only test could pass with the wrong return value.',
                        'mitigation':'Execute a faulty-return negative against the same public assertion.'}]
    design['coverage'] = [{'criterion_id':'AC-1','acceptance':intent['acceptance'][0]['statement'],
                          'delivered_by':'The public helper import and operator-visible scripts/helper-journey.py assertion implement docs/contract.md.'}]
    design_path.write_text(json.dumps(design))
    plan_path = artifacts / 'plan.json'
    plan = json.loads(plan_path.read_text())
    task = copy.deepcopy(plan['tasks'][0])
    task.update(id='public-helper',objective='Preserve the observable public helper return value.',dependencies=[],
                source_of_truth=['intent.json#/acceptance/0','docs/contract.md'],deliverables=['helper.py public import'],
                out_of_scope=['Other outcomes or engine execution'],validation=['AC-1: python3 scripts/helper-journey.py imports helper and asserts helper(7) == 7; returning None fails. Retain the executed faulty-helper negative separately from current passing proof.'],
                handoff='The external driver runs the public script and retains its actual output and source identity.',
                criterion_ids=['AC-1'],proof_command_ids=['helper-test'])
    plan.update(objective='Preserve the public helper outcome with a distinguishing external assertion and phase-specific requirement coverage.',
                tasks=[task],dependency_graph=[],proof_commands=[{'id':'helper-test','command':sys.executable,
                'args':['-B','scripts/helper-journey.py'],'owner':'external-proof-owner','obligation':'AC-1: public import assertion helper(7) == 7 under docs/contract.md.'}])
    plan_path.write_text(json.dumps(plan))
    report_path = artifacts / 'implementation-report.json'
    report = json.loads(report_path.read_text())
    report['summary'] = 'The public helper import returns the input; the operator-visible script distinguishes a faulty return.'
    report['changed_surface'] = ['scripts/helper-journey.py public assertion','docs/PRD.md fixture requirement and docs/contract.md boundary']
    report['coverage']['commit'] = log.execute(['git','rev-parse','HEAD'], cwd=repository, raw=True).stdout.strip()
    report['coverage']['documents'] = [{'path':'docs/contract.md','revision':'current'}]
    report['validation'] = [{'criterion_id':'AC-1','proof':'The operator-visible scripts/helper-journey.py assertion imports helper and checks helper(7) == 7 under docs/contract.md.'}]
    report_path.write_text(json.dumps(report))
    # Large generated checkpoint exercises real line-reader chunking, not only a tiny JSON file.
    bulk = repository / 'checkpoint-readability-toy'
    bulk.mkdir()
    for index in range(900):
        (bulk / f'entry-{index:04d}.txt').write_text('NEW toy checkpoint readability source\n')
    checker = log.execute([args.checker, '--repo', repository], cwd=repository, raw=True)
    require(checker.returncode == 0 and checker.stdout.splitlines()[0] == 'GREEN', 'Toy checker not actually GREEN')


def reconcile(ctx):
    value = {'revision':'new-toy-reconciliation','author':{'name':'new-toy-driver','kind':'agent'},
       'mode':'bookends-enabled','branch':'sufficient-existing-wording',
       'document_observations':[{'path':'docs/contract.md','status':'unchanged','observation':'NEW toy helper return contract inspected.'}],
       'behavior_observations':[{'status':'matches-intent','observation':'Actual toy helper-test result is required.'}],
       'action':'no-document-change','action_reason':'NEW toy LE-1 already demands helper outcome.',
       'authorization':'not-required','application':'not-required','commit':'not-required',
       'traceability':{'status':'retained','references':['LE-1','docs/contract.md']},
       'proof_references':['plan.json#/proof_commands'],'blockers':[],'decision':'complete'}
    (ctx['artifacts'] / 'reconciliation.json').write_text(json.dumps(value))
