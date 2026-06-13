(() => {
  const dataNode = document.getElementById("puzzle-data");
  if (!dataNode) {
    return;
  }

  const puzzle = JSON.parse(dataNode.textContent);
  const size = puzzle.size;
  const cells = Array.from(document.querySelectorAll(".cell"));
  const timerNode = document.getElementById("timer");
  const queenCountNode = document.getElementById("queen-count");
  const ruleStatusNode = document.getElementById("rule-status");
  const winDialog = document.getElementById("win-dialog");
  const winTime = document.getElementById("win-time");
  const storageKey = `queensgame:9x9:${puzzle.id}`;
  const EMPTY = 0;
  const MARK = 1;
  const QUEEN = 2;
  const AUTO_MARK = 3;
  let mode = "queen";
  let states = new Array(size * size).fill(EMPTY);
  let history = [];
  let startedAt = Date.now();
  let completed = false;
  let completedSeconds = 0;
  let validationToken = 0;
  let markDrag = null;

  const formatTime = (seconds) => {
    const minutes = Math.floor(seconds / 60);
    const rest = seconds % 60;
    return `${String(minutes).padStart(2, "0")}:${String(rest).padStart(2, "0")}`;
  };

  const elapsedSeconds = () => {
    if (completed) {
      return completedSeconds;
    }
    return Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  };

  const indexFor = (row, col) => row * size + col;

  const isMarked = (state) => state === MARK || state === AUTO_MARK;

  const queens = () =>
    states
      .map((state, index) => {
        if (state !== QUEEN) {
          return null;
        }
        return [Math.floor(index / size), index % size];
      })
      .filter(Boolean);

  const save = () => {
    localStorage.setItem(
      storageKey,
      JSON.stringify({
        states,
        startedAt,
        completed,
        completedSeconds,
      }),
    );
  };

  const load = () => {
    try {
      const saved = JSON.parse(localStorage.getItem(storageKey) || "null");
      if (!saved || !Array.isArray(saved.states) || saved.states.length !== size * size) {
        return;
      }
      states = saved.states.map((value) =>
        value === MARK || value === QUEEN || value === AUTO_MARK ? value : EMPTY,
      );
      startedAt = Number(saved.startedAt) || Date.now();
      completed = Boolean(saved.completed);
      completedSeconds = Number(saved.completedSeconds) || 0;
    } catch (_) {
      states = new Array(size * size).fill(EMPTY);
      startedAt = Date.now();
      completed = false;
      completedSeconds = 0;
    }
  };

  const setMode = (nextMode) => {
    mode = nextMode;
    document.querySelectorAll(".mode-button").forEach((button) => {
      button.classList.toggle("active", button.dataset.mode === mode);
    });
  };

  const renderCells = (conflicts = []) => {
    const conflictSet = new Set(conflicts.map(([row, col]) => `${row},${col}`));
    cells.forEach((cell) => {
      const row = Number(cell.dataset.row);
      const col = Number(cell.dataset.col);
      const state = states[indexFor(row, col)];
      cell.classList.toggle("marked", isMarked(state));
      cell.classList.toggle("auto-marked", state === AUTO_MARK);
      cell.classList.toggle("queen", state === QUEEN);
      cell.classList.toggle("conflict", conflictSet.has(`${row},${col}`));
      cell.setAttribute(
        "aria-label",
        `Row ${row + 1}, column ${col + 1}${state === QUEEN ? ", queen" : isMarked(state) ? ", marked" : ""}`,
      );
    });
  };

  const updateTimer = () => {
    timerNode.textContent = formatTime(elapsedSeconds());
  };

  const updateFromValidation = (result) => {
    queenCountNode.textContent = `${result.queen_count} / ${result.expected_queens} queens`;
    const status = [
      `${result.satisfied_rows}/${size} rows`,
      `${result.satisfied_columns}/${size} columns`,
      `${result.satisfied_regions}/${size} regions`,
    ].join(" · ");
    ruleStatusNode.textContent = result.messages.length > 0 ? `${status} - ${result.messages[0]}` : status;
    renderCells(result.conflict_cells);

    if (result.complete && !completed) {
      const finishSeconds = elapsedSeconds();
      completed = true;
      completedSeconds = finishSeconds;
      save();
      updateTimer();
      winTime.textContent = `Finished in ${formatTime(completedSeconds)}.`;
      winDialog.hidden = false;
    }
  };

  const validate = async () => {
    const token = ++validationToken;
    const response = await fetch("/api/validate", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        id: puzzle.id,
        queens: queens(),
      }),
    });

    if (!response.ok || token !== validationToken) {
      return;
    }

    updateFromValidation(await response.json());
  };

  const pushHistory = () => {
    history.push(states.slice());
    if (history.length > 100) {
      history.shift();
    }
  };

  const invalidatedByQueen = (queenRow, queenCol, row, col) => {
    if (queenRow === row && queenCol === col) {
      return false;
    }

    return (
      queenRow === row ||
      queenCol === col ||
      puzzle.regions[queenRow][queenCol] === puzzle.regions[row][col] ||
      (Math.abs(queenRow - row) === 1 && Math.abs(queenCol - col) === 1)
    );
  };

  const refreshAutoMarks = () => {
    states = states.map((state) => (state === AUTO_MARK ? EMPTY : state));

    queens().forEach(([queenRow, queenCol]) => {
      for (let row = 0; row < size; row += 1) {
        for (let col = 0; col < size; col += 1) {
          const index = indexFor(row, col);
          if (states[index] === EMPTY && invalidatedByQueen(queenRow, queenCol, row, col)) {
            states[index] = AUTO_MARK;
          }
        }
      }
    });
  };

  const commitState = (index, nextState, options = {}) => {
    if (states[index] === nextState) {
      return;
    }
    pushHistory();
    states[index] = nextState;
    if (options.refreshAutoMarks) {
      refreshAutoMarks();
    }
    completed = false;
    completedSeconds = 0;
    renderCells();
    save();
    validate();
  };

  const startMarkDrag = (index) => {
    markDrag = {
      startIndex: index,
      moved: false,
      changed: false,
      historyStarted: false,
      needsAutoRefresh: false,
    };
  };

  const setDragMark = (index) => {
    if (!markDrag || states[index] === MARK) {
      return;
    }

    if (!markDrag.historyStarted) {
      pushHistory();
      markDrag.historyStarted = true;
    }

    if (states[index] === QUEEN) {
      markDrag.needsAutoRefresh = true;
    }

    states[index] = MARK;
    markDrag.changed = true;
    renderCells();
  };

  const dragMarkCell = (index) => {
    if (!markDrag) {
      return;
    }

    if (!markDrag.moved) {
      markDrag.moved = true;
      setDragMark(markDrag.startIndex);
    }

    setDragMark(index);
  };

  const finishMarkDrag = (index = null) => {
    if (!markDrag) {
      return;
    }

    const drag = markDrag;
    markDrag = null;

    if (!drag.moved) {
      toggleMark(index ?? drag.startIndex);
      return;
    }

    if (!drag.changed) {
      return;
    }

    if (drag.needsAutoRefresh) {
      refreshAutoMarks();
    }

    completed = false;
    completedSeconds = 0;
    renderCells();
    save();
    validate();
  };

  const toggleMark = (index) => {
    const currentState = states[index];
    const nextState = isMarked(currentState) ? EMPTY : MARK;
    commitState(index, nextState, { refreshAutoMarks: currentState === QUEEN });
  };

  const toggleQueen = (index) => {
    commitState(index, states[index] === QUEEN ? EMPTY : QUEEN, { refreshAutoMarks: true });
  };

  const nextStateForMode = (currentState) => {
    if (mode === "clear") {
      return EMPTY;
    }
    if (mode === "mark") {
      return isMarked(currentState) ? EMPTY : MARK;
    }
    return currentState === QUEEN ? EMPTY : QUEEN;
  };

  const applyModeAction = (index) => {
    const currentState = states[index];
    const nextState = nextStateForMode(currentState);
    commitState(index, nextState, {
      refreshAutoMarks: currentState === QUEEN || nextState === QUEEN,
    });
  };

  cells.forEach((cell) => {
    const row = Number(cell.dataset.row);
    const col = Number(cell.dataset.col);
    const index = indexFor(row, col);

    cell.addEventListener("pointerdown", (event) => {
      if (event.pointerType === "mouse" && event.button === 0) {
        event.preventDefault();
        startMarkDrag(index);
      }
    });

    cell.addEventListener("pointerenter", (event) => {
      if (!markDrag) {
        return;
      }

      if (event.pointerType === "mouse" && (event.buttons & 1) === 1) {
        dragMarkCell(index);
      } else {
        finishMarkDrag();
      }
    });

    cell.addEventListener("pointerup", (event) => {
      if (event.pointerType === "mouse") {
        if (event.button === 0) {
          event.preventDefault();
          finishMarkDrag(index);
        }
        return;
      }

      applyModeAction(index);
    });

    cell.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      toggleQueen(index);
    });

    cell.addEventListener("keydown", (event) => {
      if (event.key === " " || event.key === "Enter") {
        event.preventDefault();
        applyModeAction(index);
      }
      if (event.key === "q" || event.key === "Q") {
        event.preventDefault();
        toggleQueen(index);
      }
      if (event.key === "x" || event.key === "X") {
        event.preventDefault();
        toggleMark(index);
      }
      if (event.key === "Backspace" || event.key === "Delete") {
        event.preventDefault();
        commitState(index, EMPTY, { refreshAutoMarks: states[index] === QUEEN });
      }
    });
  });

  window.addEventListener("pointerup", (event) => {
    if (event.pointerType === "mouse" && event.button === 0) {
      finishMarkDrag();
    }
  });

  document.querySelectorAll(".mode-button").forEach((button) => {
    button.addEventListener("click", () => setMode(button.dataset.mode));
  });

  document.getElementById("undo-button").addEventListener("click", () => {
    const previous = history.pop();
    if (!previous) {
      return;
    }
    states = previous;
    completed = false;
    completedSeconds = 0;
    renderCells();
    save();
    validate();
  });

  document.getElementById("hint-button").addEventListener("click", validate);

  document.getElementById("reset-button").addEventListener("click", () => {
    pushHistory();
    states = new Array(size * size).fill(EMPTY);
    startedAt = Date.now();
    completed = false;
    completedSeconds = 0;
    winDialog.hidden = true;
    renderCells();
    save();
    validate();
    updateTimer();
  });

  document.getElementById("close-win").addEventListener("click", () => {
    winDialog.hidden = true;
  });

  winDialog.addEventListener("click", (event) => {
    if (event.target === winDialog) {
      winDialog.hidden = true;
    }
  });

  load();
  refreshAutoMarks();
  renderCells();
  validate();
  updateTimer();
  setInterval(updateTimer, 1000);
})();
