"""Real interpreter execution, resource exhaustion and recovery tests."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[2]
subprocess.run(['make', '-C', 'ports/python', '-j4'], cwd=root, check=True, stdout=subprocess.DEVNULL)
binary = root / 'build/python/runtime-test'
subprocess.run(['gcc', '-no-pie', 'ports/python/host_test.c', 'build/python/libexpos_python.a', '-o', str(binary)], cwd=root, check=True)

def run(source, code, expected=''):
    result = subprocess.run([str(binary), source], capture_output=True, text=True, timeout=5)
    assert result.returncode == code, (source, result.returncode, result.stdout, result.stderr)
    assert expected in result.stdout, (expected, result.stdout)

run('def square(x):\n return x*x\nprint([square(i) for i in range(5)])', 0, '[0, 1, 4, 9, 16]')
run('class Counter:\n def __init__(self): self.n=3\nprint(Counter().n)', 0, '3')
run('print({"meaning": 42}["meaning"])', 0, '42')
run('while True: pass', 1)
run('while True:\n try:\n  while True: pass\n except:\n  pass', 1)
run('sum(range(1000000000))', 1)
run('a = "x" * 10000000', 3, 'MemoryError')
run('import os', 3)
run('print("x" * 20000)', 2)
run('print(', 3, 'SyntaxError')
result = subprocess.run([str(binary), 'while True: pass', 'print(42)'], capture_output=True, text=True, timeout=5)
assert result.returncode == 0 and '42' in result.stdout, result
print('EXPOS PYTHON RUNTIME TESTS PASSED')
