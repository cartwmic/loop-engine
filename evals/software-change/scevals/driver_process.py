"""Container-only launcher: idle/hard stops must kill the whole driver tree."""
import os
from pathlib import Path
import signal
import subprocess
import sys


def main():
    child = subprocess.Popen(sys.argv[1:], start_new_session=True)
    Path('/workspace/trial/driver.pid').write_text(str(child.pid))

    def stop(*_):
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        return child.wait()
    finally:
        stop()


if __name__ == '__main__':
    sys.exit(main())
