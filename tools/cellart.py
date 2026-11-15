"""Turn a cell-art SVG exported from the Figma file into a shell asset.

    python tools/cellart.py IN.svg SUBCELL_W SUBCELL_H OUT.txt [--pause]

The designer draws the shell as one path per colour layer, every path made of
rectangles on the sub-cell grid (two sub-cells across and two down per
terminal cell). The asset is that grid as text, one letter per sub-cell, plus
a second grid that names the parts that move: d-pad arms and buttons.
"""
import re
import sys
from collections import deque

LAYERS = {
    'shadow': 's', 'body': 'b', 'body-hi': 'h', 'body-lo': 'l', 'bezel': 'z', 'deep': 'd',
    'stripe-a': 'A', 'stripe-b': 'B', 'led': 'L', 'dpad': 'p', 'dpad-hi': 'P', 'dimple': 'm',
    'btn': 'n', 'btn-hi': 'N', 'btn-lo': 'o', 'slot': 'S', 'ink': 'k',
}
DARK, BRIGHT = set('pPm'), set('nNo')


def read(path, sw, sh):
    text = open(path, encoding='utf-8').read()
    width, height = (float(v) for v in re.search(r'width="([\d.]+)" height="([\d.]+)"', text).groups())
    cols, rows = round(width / sw), round(height / sh)
    grid = [['.'] * cols for _ in range(rows)]
    for name, d in re.findall(r'<path id="cell art [^"]*?(?:#183;|·) ?([a-z-]+)" d="([^"]*)"', text):
        letter = LAYERS[name]
        for x0, y0, x1, y1 in re.findall(r'M([\d.]+) ([\d.]+)H([\d.]+)V([\d.]+)H[\d.]+V[\d.]+Z', d):
            for y in range(round(float(y0) / sh), round(float(y1) / sh)):
                for x in range(round(float(x0) / sw), round(float(x1) / sw)):
                    if 0 <= y < rows and 0 <= x < cols:
                        grid[y][x] = letter
    return grid


def groups(grid, letters):
    rows, cols = len(grid), len(grid[0])
    seen, out = set(), []
    for y in range(rows):
        for x in range(cols):
            if grid[y][x] in letters and (x, y) not in seen:
                queue, cells = deque([(x, y)]), []
                seen.add((x, y))
                while queue:
                    cx, cy = queue.popleft()
                    cells.append((cx, cy))
                    for nx, ny in ((cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)):
                        if 0 <= nx < cols and 0 <= ny < rows and grid[ny][nx] in letters and (nx, ny) not in seen:
                            seen.add((nx, ny))
                            queue.append((nx, ny))
                out.append(cells)
    return out


def regions(grid):
    rows, cols = len(grid), len(grid[0])
    out = [['.'] * cols for _ in range(rows)]
    found = groups(grid, DARK | BRIGHT)
    bright = sorted((g for g in found if any(grid[y][x] in BRIGHT for x, y in g)), key=len, reverse=True)[:2]
    dark = sorted((g for g in found if not any(grid[y][x] in BRIGHT for x, y in g)), key=len, reverse=True)
    for name, group in zip('BA', sorted(bright, key=lambda g: min(x for x, _ in g))):
        for x, y in group:
            if grid[y][x] in BRIGHT:
                out[y][x] = name
    if dark:
        cross = dark[0]
        xs, ys = [x for x, _ in cross], [y for _, y in cross]
        x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
        top = [x for x, y in cross if y <= y0 + (y1 - y0) // 5]
        left = [y for x, y in cross if x <= x0 + (x1 - x0) // 5]
        vx0, vx1, hy0, hy1 = min(top), max(top), min(left), max(left)
        for x, y in cross:
            if y < hy0 and vx0 <= x <= vx1 + 2:
                out[y][x] = 'U'
            elif y > hy1 and vx0 <= x <= vx1 + 2:
                out[y][x] = 'D'
            elif x < vx0 and hy0 <= y <= hy1 + 1:
                out[y][x] = 'L'
            elif x > vx1 and hy0 <= y <= hy1 + 1:
                out[y][x] = 'R'
        pills = sorted(sorted(dark[1:], key=len, reverse=True)[:2], key=lambda g: min(x for x, _ in g))
        for name, group in zip('ET', pills):  # sElect, sTart
            for x, y in group:
                out[y][x] = name
    return out


def main():
    source, sw, sh, target = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
    grid = read(source, sw, sh)
    if '--pause' in sys.argv:
        # The design shows one menu row and one slot card selected. The app
        # draws the selection itself, so take both out of the art: the bar
        # becomes panel, and the selected card becomes a copy of its neighbour.
        for row in grid:
            row[:] = ['z' if c == 'n' else c for c in row]
        for y in range(14, 38):
            grid[y][170:240] = grid[y][94:164]
    parts = regions(grid)
    with open(target, 'w', encoding='utf-8', newline='\n') as out:
        out.write('size %d %d\n' % (len(grid[0]), len(grid)))
        out.writelines(''.join(row) + '\n' for row in grid)
        out.write('parts\n')
        out.writelines(''.join(row) + '\n' for row in parts)
    print(target, len(grid[0]), len(grid), sorted({c for row in parts for c in row}))


if __name__ == '__main__':
    main()
