// TODO double check all defaults for nullability etc
document.addEventListener("alpine:init", () => {
    window.Alpine.data("colors", () => ({
        // TODO make null
        selectedColorScheme: {
            by_color_theme: {
                unknown: {
                    colors: {
                    },
                    colordata: {
                    },
                },
            },
        },
        terminalColorTheme: "dark",
        actualTerminalBackgroundColor: "black",
        overriddenTerminalBackgroundColor: undefined,
        showSaveButton: false,
        csUserVisibleTitle: "",
        selectedColorSetting: "fish_color_normal",
        // Every colour variable the shell declares, in declaration order.
        colourVariables: window.__fish_colour_variables,
        // The variable a scope path stands for, so a classified run's path reaches its
        // variable.
        variableOfPath: Object.fromEntries(
            window.__fish_colour_variables
                .filter((variable) => variable.path)
                .map((variable) => [variable.path, variable.variable])),
        // The declaration's entry for a variable, containing the chain it resolves through.
        colourVariableOf: Object.fromEntries(
            window.__fish_colour_variables
                .map((variable) => [variable.variable, variable])),
        // The attribute's entry for a variable.
        attributeOf: Object.fromEntries(
            window.__fish_highlight_attributes
                .map((attribute) => [attribute.variable, attribute])),
        // The sample, classified by the highlighting pass itself.
        colourSample: [],
        // The run the user picked, the scopes it resolves through, and which of them the
        // picker edits.  Nothing is picked until the sample is clicked, so the sample rests
        // undecorated.
        pickedPieceIndex: null,
        pickedStops: [],
        selectedStopIndex: 0,
        hoveredStopIndex: null,
        // The variable a row of the variable list is under the pointer, if any.
        hoveredVariable: null,
        // Which field of the selected variable a colour writes.
        editingField: "foreground",
        // The pane the keyboard is in: the sample, the rail of scopes, the statement, the
        // style fields, or the colours.  Arrows move inside a pane and across panes.
        pane: "sample",
        // The style pane's two rows: the three fields, and the two flags a foreground has.
        styleRow: "fields",
        flagFocus: 0,
        // The colour pane's cells, by row and column.
        colourRow: 0,
        colourColumn: 1,
        // The statement the page shows for the selected variable, the classified runs it
        // is drawn from, and the timer that keeps a keystroke from asking the server for a
        // classification of every word in progress.  `statementRequest` keeps a slow answer
        // from landing after a newer one.
        themeStatement: "",
        statementPieces: [],
        statementRequest: 0,
        statementTimer: 0,
        statementClassifiedText: "",
        // How long a keystroke settles before the line is classified, and how much later a
        // write waits before the line it re-states is classified: long enough that typing
        // reaches the server as words, not as letters.
        statementDebounceMs: 200,
        applyingStatement: false,

        variableForPiece(piece) {
            return this.variableOfPath[piece.path];
        },

        // Pick the run at `pieceIndex`, and edit the most specific scope that colours it.
        pickPiece(pieceIndex) {
            const piece = this.colourSample[pieceIndex];
            if (!piece) return;
            this.pickedPieceIndex = pieceIndex;
            this.pickedStops = this.stopsForPiece(piece);
            this.hoveredStopIndex = null;
            this.selectStop(0);
            this.revealSelectedVariableInList();
            this.focusPane("rail");
        },

        // The scopes that colour one run, most specific first: the run's own scope, the
        // chain it resolves through, the scopes that enclose it, and the root.
        stopsForPiece(piece) {
            return this.stopsFor(this.variableForPiece(piece), piece.enclosing);
        },

        // The scopes that colour a variable, most specific first.  A variable chosen from
        // the list has no run and so no scopes around it, but the chain it resolves
        // through is the same.
        stopsFor(variable, enclosingPaths) {
            const stops = [];
            const seen = new Set();
            const add = (candidate, kind, path) => {
                if (!candidate || seen.has(candidate)) return;
                seen.add(candidate);
                const entry = this.colourVariableOf[candidate];
                stops.push({
                    variable: candidate,
                    kind,
                    path: path || (entry ? entry.path : null) || candidate,
                });
            };
            const entry = this.colourVariableOf[variable];
            add(variable, "leaf", entry ? entry.path : null);
            for (const step of entry ? entry.chain : []) {
                if (step !== "fish_color_normal") add(step, "chain", null);
            }
            for (const enclosing of (enclosingPaths || []).slice().reverse()) {
                add(this.variableOfPath[enclosing], "enclosing", enclosing);
            }
            add("fish_color_normal", "root", "source");
            return stops;
        },

        // Move the specificity stop the picker edits.
        selectStop(stopIndex) {
            const stop = this.pickedStops[stopIndex];
            if (!stop) return;
            this.selectedStopIndex = stopIndex;
            this.selectColorSetting(stop.variable);
        },

        // Walk the rail, which is what the arrows are for.
        moveStop(delta) {
            const stopIndex = this.selectedStopIndex + delta;
            if (stopIndex < 0 || stopIndex >= this.pickedStops.length) return;
            this.selectStop(stopIndex);
            this.revealSelectedVariableInList();
        },

        // A stop picked in the rail is a pick made away from the list, so the list follows it.
        selectRailStop(stopIndex) {
            this.selectStop(stopIndex);
            this.revealSelectedVariableInList();
        },

        // Whether the theme sets the stop's variable: when it does not, what that variable
        // inherits renders the label, and the rail dims it so that the overrides show.
        stopIsSet(stop) {
            return this.hasColourFields(this.currentColorScheme().colordata[stop.variable]);
        },

        selectedStop() {
            return this.pickedStops[this.selectedStopIndex];
        },

        // The scope path the picker is editing, when a stop is selected.
        selectedStopPath() {
            const stop = this.selectedStop();
            return stop ? stop.path : "";
        },

        clearPicked() {
            this.pickedPieceIndex = null;
            this.pickedStops = [];
            this.selectedStopIndex = 0;
            this.hoveredStopIndex = null;
        },

        // Select a variable from outside the sample.  There is no run to walk, but the
        // chain it resolves through is the same, so the rail opens on it.
        selectVariable(variable) {
            this.pickedPieceIndex = null;
            this.hoveredStopIndex = null;
            this.pickedStops = this.stopsFor(variable, []);
            this.selectStop(0);
            this.focusPane("rail");
        },

        // A double click on a variable jumps the sample to a run that variable colours: the
        // one nearest the sample's own middle, so the run under the eye is the one found and
        // a second double click finds it already there and moves nothing.  A run that wraps
        // is measured by the line of it nearest the middle, since a token is a line of text
        // and the box of a wrapped run spans several.
        revealVariableInSample(variable) {
            this.$nextTick(() => {
                const sample = this.$refs.sampleRows;
                if (!sample) return;
                const sampleBox = sample.getBoundingClientRect();
                const middle = sampleBox.top + sampleBox.height / 2;
                let nearest = null;
                let nearestDistance = Infinity;
                for (const run of sample.querySelectorAll(".colour_sample_piece")) {
                    const piece = this.colourSample[Number(run.dataset.pieceIndex)];
                    if (!piece || !this.resolutionOf(piece).includes(variable)) continue;
                    for (const line of run.getClientRects()) {
                        const distance = Math.abs(line.top + line.height / 2 - middle);
                        if (distance < nearestDistance) {
                            nearest = line;
                            nearestDistance = distance;
                        }
                    }
                }
                if (!nearest) return;
                sample.scrollTop += nearest.top - sampleBox.top - (sampleBox.height - nearest.height) / 2;
            });
        },

        // Hovering a row of the variable list marks every run that variable colours, the
        // same mark the rail leaves on the runs a scope covers.
        hoverVariable(variable) {
            this.hoveredVariable = variable;
        },

        clearHoveredVariable() {
            this.hoveredVariable = null;
        },

        // Whether a run is one the active stop covers: the picked run for the run's own
        // scope, every run resolving through a chain step, every run a side of the
        // enclosing scope.  A hovered variable takes over the mark while the pointer is on
        // it.  The root colours every run, so a mark from it would cover the whole sample.
        pieceInActiveStop(pieceIndex) {
            const piece = this.colourSample[pieceIndex];
            if (this.hoveredVariable) {
                if (this.hoveredVariable === "fish_color_normal") return false;
                return this.resolutionOf(piece).includes(this.hoveredVariable);
            }
            const stop = this.pickedStops[this.hoveredStopIndex ?? this.selectedStopIndex];
            if (!stop || stop.variable === "fish_color_normal") return false;
            switch (stop.kind) {
                case "leaf":
                    // A run picked in the sample is one run; a variable picked from the
                    // list is every run the pass gave that scope.
                    return this.pickedPieceIndex !== null
                        ? pieceIndex === this.pickedPieceIndex
                        : this.variableForPiece(piece) === stop.variable;
                case "enclosing":
                    return piece.enclosing.includes(stop.path);
                case "chain":
                    return this.resolutionOf(piece).includes(stop.variable);
            }
            return false;
        },

        // The style a stop's own variable renders: its value, else the chain it resolves
        // through, else the root.  An unset stop renders by what it inherits, which is what
        // a colour set at it would override.
        effectiveStyleOf(variable) {
            const colours = this.currentColorScheme().colordata;
            const entry = this.colourVariableOf[variable];
            const resolution = ["fish_color_normal"];
            for (const step of (entry ? entry.chain : []).slice().reverse()) {
                if (step !== "fish_color_normal") resolution.push(step);
            }
            resolution.push(variable);
            let named = {};
            for (const step of resolution) {
                named = this.writeFields(named, step, colours, this.channelFields);
            }
            return this.styleOf(named);
        },

        selectedSpecification() {
            return this.currentColorScheme().colordata[this.selectedColorSetting];
        },

        // The specification the picker writes: the variable's own value, or a value with no
        // field, which is what setting an override starts from.  A scope absent from the
        // theme has no field until one is set, so the sample renders it by what it
        // inherits either way.
        writtenSpecification() {
            const variable = this.selectedColorSetting;
            if (!variable) return undefined;
            const colourScheme = this.currentColorScheme();
            if (!colourScheme.colordata[variable]) {
                colourScheme.colordata[variable] = this.emptySpecification();
                colourScheme.colors[variable] = "";
            }
            return colourScheme.colordata[variable];
        },

        // The colour the grid marks as chosen, for the field it writes.
        selectedFieldColour() {
            const specification = this.selectedSpecification();
            if (!specification) return "";
            switch (this.editingField) {
                case "foreground":
                    return specification.color;
                case "background":
                    return specification.background;
                case "underline_colour":
                    return specification["underline-color"];
            }
            return "";
        },

        // What the picker is setting: the field of the selected variable, and the colour
        // that field has now.
        fieldTitle() {
            switch (this.editingField) {
                case "background":
                    return "a background color";
                case "underline_colour":
                    return "an underline color";
            }
            return "a foreground color";
        },

        selectField(field) {
            this.editingField = field;
            this.styleRow = "fields";
        },

        selectedFlag(flag) {
            const specification = this.selectedSpecification();
            return !!(specification && specification[flag]);
        },

        // Switch one flag of the selected variable on or off.  The flags are one field of a
        // `set_color` value, so the value has all of them, and the line the page shows is
        // re-derived from the value.
        toggleFlag(flag) {
            const specification = this.writtenSpecification();
            if (!specification) return;
            if (flag === "underline") {
                specification.underline = specification.underline ? null : "single";
            } else {
                specification[flag] = !specification[flag];
            }
            this.noteThemeChanged();
            this.refreshThemeStatement();
        },

        // The page's own text colour, which a variable with no foreground inherits.
        defaultTextColour() {
            return "#" + text_color_for_color(this.actualTerminalBackgroundColor);
        },

        // The controls sit on the background of the sample preview, so that they are part of
        // the preview rather than swatches of their own.
        previewBackgroundColour() {
            return this.actualTerminalBackgroundColor;
        },

        // The `Aa` control of each field, drawn in the field's own colour on the preview's
        // background.
        foregroundButtonStyle() {
            const style = this.effectiveStyleOf(this.selectedColorSetting);
            return {
                "color": style.color || this.defaultTextColour(),
                "background-color": this.previewBackgroundColour(),
            };
        },

        // The background control is the one control whose chip is the field itself.
        backgroundButtonStyle() {
            const style = this.effectiveStyleOf(this.selectedColorSetting);
            const background = style["background-color"] || this.previewBackgroundColour();
            return {
                "color": contrasting_color(background, this.defaultTextColour()),
                "background-color": background,
            };
        },

        underlineButtonStyle() {
            const style = this.effectiveStyleOf(this.selectedColorSetting);
            const specification = this.selectedSpecification();
            const underlined = !!(specification && specification.underline);
            // The glyph shows the underline's own color when it has one, so that the
            // control reads as the underline rather than a second foreground.
            const glyph =
                (underlined && interpret_color(specification["underline-color"])) ||
                style.color ||
                this.defaultTextColour();
            return {
                "color": glyph,
                "background-color": this.previewBackgroundColour(),
                "text-decoration": underlined ? "underline " + css_underline_style(specification.underline) : "none",
                "text-decoration-color": interpret_color(specification ? specification["underline-color"] : ""),
            };
        },

        // The style a piece of the sample resolves to, composed as the shell composes it:
        // the root, the enclosing scopes from the outermost, the leaf's chain from its
        // least specific step, the leaf, and then its attributes.  Each variable's value
        // contributes its own fields, a more specific one after a less specific one, and
        // `--reset` clears the fields before it writes.
        styleForPiece(piece) {
            const colours = this.currentColorScheme().colordata;
            const resolution = this.resolutionOf(piece);
            let named = {};
            for (const variable of resolution) {
                named = this.writeFields(named, variable, colours, this.channelFields);
            }
            for (const variable of piece.attributes) {
                named = this.writeAttribute(named, variable, colours);
            }
            return this.styleOf(named);
        },

        // The fields each channel composes: the foreground channel takes the flags and
        // the underline colour with it, and the background channel is its own.
        channelFields: {
            foreground: ["foreground", "underline_colour", "flags"],
            background: ["background"],
        },

        // The variables one piece resolves through, least specific first.
        resolutionOf(piece) {
            const leaf = this.colourVariableOf[this.variableForPiece(piece)];
            if (!leaf) return [];
            const root = "fish_color_normal";
            const resolution = [root];
            for (const enclosing of piece.enclosing.slice().reverse()) {
                const variable = this.variableOfPath[enclosing];
                if (variable) resolution.push(variable);
            }
            for (const step of leaf.chain.slice().reverse()) {
                if (step !== root) resolution.push(step);
            }
            resolution.push(leaf.variable);
            return resolution;
        },

        // The fields a value has.
        namedFieldsOf(specification) {
            const named = {};
            if (!specification) return named;
            if (specification.color) named.foreground = specification.color;
            if (specification.background) named.background = specification.background;
            if (specification["underline-color"])
                named.underline_colour = specification["underline-color"];
            const flags = {};
            for (const flag of ["bold", "italics", "underline", "dim", "reverse", "strikethrough"]) {
                if (specification[flag]) flags[flag] = specification[flag];
            }
            if (Object.keys(flags).length > 0) named.flags = flags;
            return named;
        },

        // One variable's fields written over the style for both channels.
        writeFields(named, variable, colours, fieldsByChannel) {
            const specification = colours[variable];
            for (const [channel, fields] of Object.entries(fieldsByChannel)) {
                if (specification && specification.reset) {
                    for (const field of fields) delete named[field];
                }
                const namedFields = this.namedFieldsOf(specification);
                for (const field of fields) {
                    if (namedFields[field] !== undefined) named[field] = namedFields[field];
                }
            }
            return named;
        },

        // One attribute's fields written over the style: a merge writes its own fields, and
        // a replace puts the terminal default in the fields it does not.
        writeAttribute(named, variable, colours) {
            const attribute = this.attributeOf[variable];
            if (!attribute) return named;
            const specification = colours[variable];
            const namedFields = this.namedFieldsOf(specification);
            if (attribute.mode === "merge" && !specification) return named;
            for (const field of attribute.fields) {
                if (attribute.mode === "merge" && specification.reset) delete named[field];
                if (attribute.mode === "replace" && namedFields[field] === undefined) {
                    delete named[field];
                }
                if (namedFields[field] !== undefined) named[field] = namedFields[field];
            }
            return named;
        },

        // The CSS properties a composed style has.
        styleOf(named) {
            const css = {};
            let foreground = named.foreground;
            let background = named.background;
            const flags = named.flags || {};
            if (flags.reverse) [foreground, background] = [background, foreground];
            if (foreground) css["color"] = interpret_color(foreground);
            if (background) css["background-color"] = interpret_color(background);
            if (flags.bold) css["font-weight"] = "bold";
            if (flags.italics) css["font-style"] = "italic";
            if (flags.dim) css["opacity"] = "0.6";
            const decorations = [];
            if (flags.underline) {
                decorations.push("underline " + css_underline_style(flags.underline));
            }
            if (flags.strikethrough) decorations.push("line-through");
            if (decorations.length) css["text-decoration"] = decorations.join(" ");
            if (named.underline_colour) {
                css["text-decoration-color"] = interpret_color(named.underline_colour);
            }
            return css;
        },

        // The title of a setting: a declared variable has the declaration's description,
        // and a variable the declaration lacks has its spelling.
        titleForSetting(variable) {
            const entry = this.colourVariables.find(
                (candidate) => candidate.variable === variable);
            return entry ? entry.description : user_visible_title_for_setting_name(variable);
        },

        text_color_for_color: window.text_color_for_color,

        currentColorScheme() {
            let scheme = this.selectedColorScheme.by_color_theme;
            if (scheme[this.terminalColorTheme] !== undefined) {
                return scheme[this.terminalColorTheme];
            }
            return scheme['unknown'];
        },

        changedSelectedColorSchemeOrColorTheme() {
            this.selectedColorScheme.name_with_color_theme =
                color_scheme_respects_color_theme(this.selectedColorScheme)
                ? `${this.selectedColorScheme.name} (${this.terminalColorTheme} color theme)`
                : this.selectedColorScheme.name;
        },

        changeSelectedColorScheme(newScheme) {
            this.terminalColorTheme = newScheme.terminal_color_theme ||
                (newScheme.by_color_theme.light !== undefined ? "light" : "dark");
            this.selectedColorScheme = newScheme;
            let preferred_background = this.currentColorScheme().preferred_background;
            if (preferred_background) {
                this.actualTerminalBackgroundColor = preferred_background;
            }
            this.overriddenTerminalBackgroundColor = undefined;
            this.changedSelectedColorSchemeOrColorTheme();
            this.clearPicked();
            this.pickedStops = this.stopsFor("fish_color_normal", []);
            this.selectedStopIndex = 0;
            this.selectColorSetting("fish_color_normal");
            this.focusPane("sample");
            //TODO: Save button should be shown only when colors are changed
            this.showSaveButton = true;

            this.noteThemeChanged();
        },

        text_color_for_color: text_color_for_color,

        border_color_for_color: border_color_for_color,

        interpret_color: interpret_color,

        // The 256-colour table the page offers, in rows of 32.
        get term256Rows() {
            return get_colors_as_nested_array(term_256_colors, 32);
        },

        // The colour pane's cells, top to bottom: the cell that clears the scope, the swatch
        // that opens a colour picker for the field the style pane writes, the value's hex
        // beside it, the terminal's sixteen colours in two rows of eight, then the
        // 256-colour table.  The hex is a readout and the arrows step over it.
        colourRows() {
            const rows = [];
            rows.push([
                { kind: "unset" },
                { kind: "swatch" },
                { kind: "hex" },
            ]);
            // The terminal's sixteen cells have palette names, so the colour of each slot is
            // the terminal's; the table below them has RGB, which fish quantises when the
            // terminal cannot show it.
            rows.push(term_16_names.slice(0, 8).map((colour_name, index) => ({
                kind: "colour", colour: colour_name, chip: term_16_colors[index],
            })));
            rows.push(term_16_names.slice(8).map((colour_name, index) => ({
                kind: "colour", colour: colour_name, chip: term_16_colors[index + 8],
            })));
            for (const row of this.term256Rows) {
                rows.push(row.map((colour) => ({ kind: "colour", colour })));
            }
            return rows;
        },

        // The cell the colour pane points at.
        focusedColourCell() {
            const rows = this.colourRows();
            const row = rows[Math.min(this.colourRow, rows.length - 1)] || [];
            return row[Math.min(this.colourColumn, Math.max(row.length - 1, 0))];
        },

        // Whether the colour pane points at a cell.
        colourCellFocused(rowIndex, columnIndex) {
            return this.colourRow === rowIndex && this.colourColumn === columnIndex;
        },

        // The style of a cell of the colour pane: a swatch gets its colour, the value's hex
        // is a readout, and the cell under the keyboard gets the selection's outline.
        colourCellStyle(cell, rowIndex, columnIndex) {
            const focused = this.colourCellFocused(rowIndex, columnIndex);
            const style = {};
            if (cell.kind === "colour") {
                style["background-color"] = interpret_color(cell.chip || cell.colour);
                if (this.selectedFieldColour() === cell.colour) {
                    style["outline"] = "2px solid " + this.defaultTextColour();
                    style["outline-offset"] = "-2px";
                }
            } else if (cell.kind === "hex") {
                style["color"] = this.defaultTextColour();
            }
            if (focused) Object.assign(style, this.selectionOutlineStyle());
            return style;
        },

        // The hex the page states for a colour value.  A value spelled as hex may carry the
        // `#` the theme file omits, and a palette name has no exact RGB at all: the hex
        // beside it is the approximation the page uses everywhere.
        hexOf(colour) {
            return colour ? interpret_color(colour.replace(/^#/, "")) : "";
        },

        // The hex the current field's colour has, as the page can state it.
        selectedFieldHex() {
            return this.hexOf(this.selectedFieldColour());
        },

        // The value the colour picker starts from when it opens.
        colourSwatchValue() {
            return this.selectedFieldHex() || "#000000";
        },

        // The picker running while the colour changes: the theme takes the colour at once,
        // and the line the theme file has for the scope follows when the picker closes.  The
        // colour is stated the way the theme file states it, which is hex without the `#`
        // the picker's value carries.
        liveSwatchColour(event) {
            this.changeSelectedColor(event.target.value.replace(/^#/, ""), false);
        },

        // The picker closing: the line is re-derived from the value.
        commitSwatchColour() {
            this.noteThemeChanged();
            this.refreshThemeStatement();
        },

        // The fields the style pane offers, in the order it lists them.
        styleFields() {
            return ["foreground", "background", "underline_colour"];
        },

        // The letters a field's control shows: the field's own, in angle brackets when the
        // scope has a value for it and square ones when it has none.
        fieldLabel(field) {
            const letters = {
                foreground: "FG",
                background: "BG",
                underline_colour: "UL",
            }[field];
            return this.fieldActive(field) ? "<" + letters + ">" : "[" + letters + "]";
        },

        // The letters of a field's label, without the brackets the field's state adds.
        fieldLetters(field) {
            return {
                foreground: "FG",
                background: "BG",
                underline_colour: "UL",
            }[field];
        },

        // Whether the selected scope has a value for the field.
        fieldActive(field) {
            const specification = this.selectedSpecification();
            if (!specification) return false;
            switch (field) {
                case "foreground":
                    return !!specification.color;
                case "background":
                    return !!specification.background;
                case "underline_colour":
                    return !!specification.underline;
            }
            return false;
        },

        // The flags the style pane lists, and which of them the keyboard points at.
        flagRows() {
            return ["bold", "italics"];
        },

        flagTitle(flag) {
            return flag === "bold" ? "Bold" : "Italics";
        },

        // The flags belong to the foreground: a background and an underline have none, so
        // the rows appear while the foreground is the field the picker writes.
        flagRowsShown() {
            return this.editingField === "foreground";
        },

        focusedFlag() {
            return this.flagRows()[this.flagFocus] || "bold";
        },

        // The style of a field's control, with the selection over it while the field is the
        // picker's or the keyboard's.
        fieldButtonStyle(field) {
            switch (field) {
                case "foreground":
                    return this.withSelection(this.foregroundButtonStyle(), this.editingField === field);
                case "background":
                    return this.withSelection(this.backgroundButtonStyle(), this.editingField === field);
                case "underline_colour":
                    return this.withSelection(this.underlineButtonStyle(), this.editingField === field);
            }
            return {};
        },

        // The style of a flag's row: the scope's own colours, with the selection's colours over
        // them while the row is the one the picker points at, which it stays while the keyboard
        // is elsewhere.  A flag that is simply on is no selection: the marker states it and the
        // word carries the flag's own font variant, which `flagTitleStyle` states.
        flagRowStyle(flag, index) {
            const style = this.effectiveStyleOf(this.selectedColorSetting);
            const row = {
                color: style.color || this.defaultTextColour(),
                "background-color": this.previewBackgroundColour(),
            };
            const selected =
                this.editingField === "foreground" && this.styleRow === "flags" && this.flagFocus === index;
            if (!selected) return row;
            const selection = this.selectionStyle();
            return {
                ...row,
                color: selection.color || row.color,
                "background-color": selection["background-color"] || row["background-color"],
            };
        },

        // The text of a flag's row carries the flag itself: the word is set bold while the
        // bold flag is on, and italics while the italics flag is on, and plain otherwise.
        flagTitleStyle(flag) {
            const style = {};
            if (flag === "bold") style["font-weight"] = this.selectedFlag("bold") ? "bold" : "normal";
            if (flag === "italics") style["font-style"] = this.selectedFlag("italics") ? "italic" : "normal";
            return style;
        },

        // The selection style of the theme: `fish_color_selection`, marking the item the
        // keyboard is on and the field the picker writes.
        selectionStyle() {
            return this.effectiveStyleOf("fish_color_selection");
        },

        // The outline the selection puts round a swatch, which stays visible when the swatch
        // has a colour of its own.
        selectionOutlineStyle() {
            const selection = this.selectionStyle();
            return {
                outline: "2px solid " + (selection.color || this.defaultTextColour()),
                "outline-offset": "1px",
            };
        },

        // One control's style with the selection's colours over it.
        withSelection(style, selected) {
            if (!selected) return style;
            const selection = this.selectionStyle();
            return {
                ...style,
                color: selection.color || style.color,
                "background-color": selection["background-color"] || style["background-color"],
                "font-weight": selection["font-weight"] || style["font-weight"],
                "font-style": selection["font-style"] || style["font-style"],
                "text-decoration": selection["text-decoration"] || style["text-decoration"],
            };
        },

        // The pane the keyboard is in takes the focus, so that the arrows reach it.
        focusPane(pane) {
            this.pane = pane;
            this.$nextTick(() => {
                const refs = {
                    sample: this.$refs.samplePane,
                    rail: this.$refs.specificity,
                    statement: this.$refs.statementInput,
                    style: this.$refs.stylePane,
                    colour: this.$refs.colourPane,
                };
                const ref = refs[pane];
                if (ref && typeof ref.focus === "function") ref.focus();
            });
        },

        // The keyboard: arrows move inside a pane and across panes, space and enter
        // activate what the keyboard is on, and escape returns to the sample.  A key the
        // pane takes is consumed, so that the page does not scroll as well; a key it does
        // not take stays the browser's, which is what keeps the caret and the text in the
        // editor working.
        onPaneKey(event) {
            let consumed = false;
            switch (this.pane) {
                case "sample":
                    consumed = this.onSampleKey(event.key);
                    break;
                case "rail":
                    consumed = this.onRailKey(event.key);
                    break;
                case "statement":
                    consumed = this.onStatementKey(event.key);
                    break;
                case "style":
                    consumed = this.onStyleKey(event.key);
                    break;
                case "colour":
                    consumed = this.onColourKey(event.key);
                    break;
            }
            if (consumed) event.preventDefault();
        },

        onSampleKey(key) {
            switch (key) {
                case "ArrowUp":
                case "ArrowRight":
                    this.focusPane("rail");
                    return true;
                case "ArrowDown":
                    this.focusPane("statement");
                    return true;
            }
            return false;
        },

        onRailKey(key) {
            switch (key) {
                case "ArrowUp":
                    this.moveStop(-1);
                    return true;
                case "ArrowDown":
                    if (this.selectedStopIndex >= this.pickedStops.length - 1) this.focusPane("statement");
                    else this.moveStop(1);
                    return true;
                case "ArrowRight":
                    this.focusPane("statement");
                    return true;
                case "ArrowLeft":
                case "Escape":
                    this.focusPane("sample");
                    return true;
            }
            return false;
        },

        // The editor keeps the keys that edit it: the horizontal arrows move the caret, and
        // space and enter belong to the text.  The vertical arrows leave the pane, because
        // the line has one row to move through.
        onStatementKey(key) {
            switch (key) {
                case "ArrowUp":
                    this.focusPane("rail");
                    return true;
                case "ArrowDown":
                    this.focusPane("style");
                    return true;
                case "Escape":
                    this.focusPane("sample");
                    return true;
            }
            return false;
        },

        onStyleKey(key) {
            if (this.styleRow === "fields") {
                const fields = this.styleFields();
                const index = Math.max(fields.indexOf(this.editingField), 0);
                switch (key) {
                    case "ArrowLeft":
                        if (index === 0) this.focusPane("sample");
                        else this.selectField(fields[index - 1]);
                        return true;
                    case "ArrowRight":
                        if (index === fields.length - 1) this.focusPane("colour");
                        else this.selectField(fields[index + 1]);
                        return true;
                    case "ArrowDown":
                        if (this.flagRowsShown()) {
                            this.styleRow = "flags";
                            this.flagFocus = 0;
                        } else {
                            this.focusPane("colour");
                        }
                        return true;
                    case "ArrowUp":
                        this.focusPane("statement");
                        return true;
                    case "Enter":
                    case " ":
                        this.focusPane("colour");
                        return true;
                }
            } else {
                switch (key) {
                    case "ArrowUp":
                        if (this.flagFocus === 0) this.styleRow = "fields";
                        else this.flagFocus -= 1;
                        return true;
                    case "ArrowDown":
                        if (this.flagFocus >= this.flagRows().length - 1) this.focusPane("colour");
                        else this.flagFocus += 1;
                        return true;
                    case "ArrowLeft":
                        this.styleRow = "fields";
                        return true;
                    case "ArrowRight":
                        this.focusPane("colour");
                        return true;
                    case "Enter":
                    case " ":
                        this.toggleFlag(this.focusedFlag());
                        return true;
                }
            }
            return false;
        },

        onColourKey(key) {
            const rows = this.colourRows();
            switch (key) {
                case "ArrowLeft": {
                    const column = this.stepColourColumn(this.colourColumn, -1);
                    if (column === null) this.focusPane("style");
                    else this.colourColumn = column;
                    return true;
                }
                case "ArrowRight": {
                    const column = this.stepColourColumn(this.colourColumn, 1);
                    if (column !== null) this.colourColumn = column;
                    return true;
                }
                case "ArrowUp":
                    if (this.colourRow === 0) {
                        this.focusPane("style");
                    } else {
                        this.colourRow -= 1;
                        this.keepColourColumn();
                    }
                    return true;
                case "ArrowDown":
                    if (this.colourRow < rows.length - 1) {
                        this.colourRow += 1;
                        this.keepColourColumn();
                    }
                    return true;
                case "Enter":
                case " ":
                    this.activateColourCell(this.focusedColourCell());
                    return true;
                case "Escape":
                    this.focusPane("sample");
                    return true;
            }
            return false;
        },

        // The column the arrows move to, stepping over the hex readout: it reports the value
        // and is not a cell the keyboard stops on.
        stepColourColumn(from, delta) {
            const rows = this.colourRows();
            const row = rows[Math.min(this.colourRow, rows.length - 1)] || [];
            let column = from + delta;
            while (column >= 0 && column < row.length && row[column].kind === "hex") {
                column += delta;
            }
            return column >= 0 && column < row.length ? column : null;
        },

        // Keep the column inside the row the keyboard moved to, and off the hex readout.
        keepColourColumn() {
            const rows = this.colourRows();
            const row = rows[Math.min(this.colourRow, rows.length - 1)] || [];
            let column = Math.min(this.colourColumn, Math.max(row.length - 1, 0));
            while (column > 0 && row[column].kind === "hex") column -= 1;
            this.colourColumn = column;
        },

        // Activate one cell of the colour pane: the clear cell unsets the scope, the swatch
        // opens a colour picker for the field the style pane writes, and a colour cell sets
        // that colour.  The hex beside the swatch is a readout and does nothing.
        activateColourCell(cell) {
            if (!cell) return;
            switch (cell.kind) {
                case "unset":
                    this.unsetSelectedScope();
                    break;
                case "swatch":
                    if (this.$refs.colourSwatch) this.$refs.colourSwatch.click();
                    break;
                case "colour":
                    this.changeSelectedColor(cell.colour);
                    break;
            }
        },

        // Clear the selected scope: the variable is unset, so the theme file has no line for
        // it and what it inherits renders it.  An empty variable would still be a line, and
        // a line is what clearing removes.
        unsetSelectedScope() {
            const scheme = this.currentColorScheme();
            delete scheme.colordata[this.selectedColorSetting];
            delete scheme.colors[this.selectedColorSetting];
            this.noteThemeChanged();
            this.refreshThemeStatement();
        },

        // A specification with no field: the value a scope starts from before a field is set.
        emptySpecification() {
            return {
                name: this.selectedColorSetting,
                color: "",
                background: "",
                "underline-color": "",
                bold: false,
                underline: null,
                italics: false,
                dim: false,
                reverse: false,
                strikethrough: false,
                reset: false,
            };
        },

        // Whether a specification has a field.  A value with no field overrides nothing, so
        // the rail renders it like an unset variable.
        hasColourFields(specification) {
            return !!specification && Object.keys(this.namedFieldsOf(specification)).length > 0;
        },

        // The words of the selected variable's line.  A variable absent from the theme has
        // no line, so there are no words to show.
        statementWords() {
            const scheme = this.currentColorScheme();
            if (!Object.prototype.hasOwnProperty.call(scheme.colordata, this.selectedColorSetting)) {
                return [];
            }
            return ["set", "--global", this.selectedColorSetting].concat(
                fish_colour_words(scheme.colordata[this.selectedColorSetting]));
        },

        // Re-derive the selected variable's line and re-colour it.
        refreshThemeStatement() {
            if (this.applyingStatement) return;
            this.themeStatement = this.statementWords().join(" ");
            this.refreshStatementPieces();
        },

        // Classify the line by the pass the sample is classified by, so the line is coloured
        // from the theme the page is editing rather than from one on disk.  The editor over
        // the coloured line is invisible text, so a keystroke has to reach this
        // classification, or the editor shows nothing; a keystroke settles for
        // `statementDebounceMs` first, so a word in progress asks for one classification
        // rather than one per letter.
        refreshStatementPieces() {
            const text = this.themeStatement;
            if (text === this.statementClassifiedText) return;
            const request = ++this.statementRequest;
            window.clearTimeout(this.statementTimer);
            this.statementTimer = window.setTimeout(async () => {
                const response = await fetch("code_styles/?text=" + encodeURIComponent(text));
                const pieces = await response.json();
                if (request !== this.statementRequest) return;
                this.statementClassifiedText = text;
                this.statementPieces = pieces;
            }, this.statementDebounceMs);
        },

        // The runs the line is drawn from.  The classified runs describe the line as it last
        // settled, so text typed since then is drawn in the style of the run it continues: a
        // quote still open renders what follows it as its own text, and the classification
        // that follows the keystrokes puts the proper styles on whatever the text turned out
        // to be.  The runs always add up to the line, so the invisible editor under them never
        // drifts from what is drawn.
        statementViewPieces() {
            const text = this.themeStatement;
            const classified = this.statementPieces;
            if (!classified.length) {
                return text ? [{ text, piece: null }] : [];
            }
            let classifiedText = "";
            for (const piece of classified) classifiedText += piece.text;
            if (classifiedText === text) {
                return classified.map((piece) => ({ text: piece.text, piece }));
            }
            let shared = 0;
            const limit = Math.min(classifiedText.length, text.length);
            while (shared < limit && classifiedText[shared] === text[shared]) shared += 1;
            const runs = [];
            let start = 0;
            let continuation = null;
            for (const piece of classified) {
                const end = start + piece.text.length;
                if (end <= shared) {
                    runs.push({ text: piece.text, piece });
                    continuation = piece;
                } else if (start < shared) {
                    runs.push({ text: piece.text.slice(0, shared - start), piece });
                    continuation = piece;
                }
                start = end;
                if (start >= shared) break;
            }
            const typedSince = text.slice(shared);
            if (typedSince) runs.push({ text: typedSince, piece: continuation });
            return runs;
        },

        // Read the line back into the picker when the editor loses the focus.  A line the
        // page cannot parse stays as typed: the editor is where the user is working, and
        // only a line that sets a colour variable reaches the theme.
        applyThemeStatement() {
            const words = split_fish_words(this.themeStatement);
            if (words.length < 3 || words[0] !== "set") {
                this.refreshStatementPieces();
                return;
            }
            let index = 1;
            while (index < words.length && words[index].startsWith("-")) index += 1;
            const variable = words[index];
            if (!variable || !this.colourVariableOf[variable]) {
                this.refreshStatementPieces();
                return;
            }
            this.applyingStatement = true;
            if (variable !== this.selectedColorSetting) {
                this.selectColorSetting(variable);
                this.revealSelectedVariableInList();
            }
            const specification = parse_fish_colour(words.slice(index + 1));
            const scheme = this.currentColorScheme();
            scheme.colordata[variable] = { name: variable, ...specification };
            scheme.colors[variable] = specification.color;
            this.applyingStatement = false;
            this.refreshStatementPieces();
            this.noteThemeChanged();
        },

        // A keystroke in the editor re-colours the line under it.  Reading the line back into
        // the picker is what losing the focus does.
        onStatementInput() {
            this.refreshStatementPieces();
        },

        // The style of one scope path, for the line's own text.
        styleForPath(path) {
            const variable = this.variableOfPath[path];
            return variable ? this.effectiveStyleOf(variable) : {};
        },

        // Change the scope the picker edits.  The rail, the line and the style pane read
        // from it, so one call re-derives the page's panes.
        selectColorSetting(variable) {
            this.selectedColorSetting = variable;
            this.csUserVisibleTitle = this.titleForSetting(variable);
            this.refreshThemeStatement();
        },

        // A pick made away from the list brings the picked scope to the middle of the list, so
        // the scope the picker writes is visible whether it came from the sample, from the
        // rail or from a line typed in the statement.  A pick made in the list itself moves
        // nothing, since the row is under the pointer already.
        revealSelectedVariableInList() {
            this.scrollVariableListTo(this.selectedColorSetting);
        },

        // Bring the picked scope's row to the middle of the list.  The list clamps the scroll
        // at its ends, which is as close to the middle as the first and last rows can come.
        // Only the list scrolls: the page stays where it is.
        scrollVariableListTo(variable) {
            this.$nextTick(() => {
                const list = this.$refs.variableList;
                if (!list) return;
                const row = list.querySelector('[data-variable="' + CSS.escape(variable) + '"]');
                if (!row) return;
                const rowBox = row.getBoundingClientRect();
                const listBox = list.getBoundingClientRect();
                list.scrollTop += rowBox.top - listBox.top - (listBox.height - rowBox.height) / 2;
            });
        },

        // Change the colour the field the style pane writes has.  A picker still running
        // passes `serialise` false: the theme takes the colour at once, and the line the
        // theme file has for the scope follows when the picker closes.
        changeSelectedColor(color, serialise = true) {
            const specification = this.writtenSpecification();
            if (!specification) return;
            switch (this.editingField) {
                case "foreground":
                    specification.color = color;
                    break;
                case "background":
                    specification.background = color;
                    break;
                case "underline_colour":
                    specification["underline-color"] = color;
                    // A colour under a line is a line: a colour set here turns the underline
                    // on, and the line carries the kind beside the colour.
                    if (!specification.underline) specification.underline = "single";
                    break;
            }
            this.currentColorScheme().colors[this.selectedColorSetting] = specification.color;
            this.noteThemeChanged();
            if (serialise) this.refreshThemeStatement();
        },

        sampleTerminalBackgroundColors: [
            "white",
            "#" + solarized.base3,
            "#300",
            "#003",
            "#" + solarized.base03,
            "#232323",
            "#" + nord.nord0,
            "black",
        ],

        /* Array of FishColorSchemes */
        colorSchemes: [],

        saveThemeButtonTitle: "Set Theme",
        // What the server said about the last write: a refused variable, or nothing.
        themeSaveMessage: "",

        // The style the refusal is shown in: the theme's own error colour, so that it reads
        // as the page's warning rather than as another chip.
        refusalStyle() {
            const error = this.effectiveStyleOf("fish_color_error");
            return { color: error.color || this.defaultTextColour() };
        },

        noteThemeChanged() {
            this.saveThemeButtonTitle = "Set Theme";
            this.themeSaveMessage = "";
        },

        async setTheme() {
            var postdata = {
                theme: this.selectedColorScheme["name"],
                colors: [],
            };
            for (let [name, value] of Object.entries(this.currentColorScheme().colordata)) {
                postdata.colors.push({
                    what: name,
                    color: value,
                });
            }
            let resp = await fetch("set_color/", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(postdata),
            });
            if (!resp.ok) {
                this.saveThemeButtonTitle = "Set Theme";
                this.themeSaveMessage = "the request failed";
                return;
            }
            // The server validates what it writes, because the file it writes is fish code.
            // A refusal comes back here, and the page states it: the variable keeps what it
            // had, and the rest of the theme is written.
            const result = await resp.json();
            const refused = result.refused || [];
            if (refused.length) {
                this.saveThemeButtonTitle = "Set Theme";
                this.themeSaveMessage = refused
                    .map((entry) => entry.variable + ": " + entry.refusal)
                    .join("; ");
                return;
            }
            this.saveThemeButtonTitle = "Theme Set!";
            this.themeSaveMessage = "";
        },

        // The prompt is a glyph, not a number of columns: `❯` is wide in some fonts and
        // narrow in others.  Its rendered width is measured once the pane is in the
        // document, and again whenever the pane's box changes, so the line keeps starting
        // under the comment after a resize or a font change.
        statementIndent: 0,
        // The observer that re-measures the prompt when the pane's box changes.
        statementPromptObserver: null,

        measureStatementIndent() {
            const prompt = this.$refs.statementPrompt;
            if (!prompt) return;
            this.statementIndent = prompt.getBoundingClientRect().width;
        },

        // The indent the line starts at, as CSS.
        statementIndentStyle() {
            return { "--statement-indent": this.statementIndent + "px" };
        },

        // The comment above the line: it names the scope the line states, in backticks, so
        // it reads as the note the sketch puts over the line.
        commentLine() {
            const path = this.selectedStopPath() ||
                (this.colourVariableOf[this.selectedColorSetting] || {}).path ||
                this.selectedColorSetting;
            return "# Colour for `" + this.csUserVisibleTitle + "` (the `" + path + "` scope):";
        },

        // Watch the prompt's box: a resize, a font change or a zoom changes the width the
        // line has to be indented by.  The prompt itself is inline, and a resize observer
        // reports nothing for an inline box, so the observer watches the pane the prompt
        // sits in, whose box changes with either.
        observeStatementPrompt() {
            this.measureStatementIndent();
            if (!this.statementPromptObserver) {
                this.statementPromptObserver = new ResizeObserver(() => this.measureStatementIndent());
            }
            if (this.$refs.statementPane) {
                this.statementPromptObserver.observe(this.$refs.statementPane);
            }
        },

        async init() {
            // The prompt is in the document before anything is fetched, so its width is
            // measured before the line can have anything to draw.
            this.$nextTick(() => this.observeStatementPrompt());
            let schemes = await (await fetch("colors/")).json();
            this.colourSample = await (await fetch("colour_sample/")).json();

            for (var scheme of schemes) {
                var currentScheme = {
                    name: "Current",
                    by_color_theme: {
                        'unknown': {
                            colors: {},
                            colordata: {},
                        }
                    },
                };
                let name = scheme["theme"];
                if (scheme["name"]) name = scheme["name"];
                currentScheme["name"] = name;
                currentScheme["name_with_color_theme"] = name;
                currentScheme["name_with_color_themes"] = name +
                    (color_scheme_respects_color_theme(scheme) ? " (light/dark)" : "");
                if (scheme["url"]) currentScheme["url"] = scheme["url"];
                var by_theme = scheme["by_color_theme"];
                for (const [color_theme, data] of Object.entries(by_theme)) {
                    let outdata = currentScheme.by_color_theme[color_theme] = {
                        colors: {},
                        colordata: {},
                        preferred_background: "black",
                    };
                    let preferredBackground = data["preferred_background"];
                    if (preferredBackground) {
                        outdata.preferred_background =
                            preferredBackground.match(/^([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/)
                            ? "#"+preferredBackground : preferredBackground;
                    }
                    let colors = data.colors;
                    for (var i in colors) {
                        outdata.colors[colors[i].name] = interpret_color(colors[i].color).replace(/#/, "");
                        // HACK: For some reason the colors array is cleared later
                        // So we cheesily encode the actual objects in colordata, so we can send them.
                        // TODO: We should switch to keeping the objects, and also displaying them
                        // with underlines and such.
                        outdata.colordata[colors[i].name] = colors[i];
                    }
                }
                this.colorSchemes.push(currentScheme);
            }
            this.changeSelectedColorScheme(JSON.parse(JSON.stringify(this.colorSchemes[0])));

            this.$watch('overriddenTerminalBackgroundColor', (newValue) => {
                if (newValue === undefined)
                    return;
                // see sampleTerminalBackgroundColors
                const isLight = newValue === 'white' || newValue === "#"+solarized.base3;
                this.terminalColorTheme = isLight ? 'light' : 'dark';
                this.actualTerminalBackgroundColor = newValue;
                this.changedSelectedColorSchemeOrColorTheme();
            });
        },
    }));

    window.Alpine.data("prompt", () => ({
        selectedPrompt: null,
        showSaveButton: true,
        samplePrompts: [],
        savePromptButtonTitle: "Set Prompt",
        // where is this initialized in the original code?
        actualTerminalBackgroundColor: "black",
        overriddenTerminalBackgroundColor: undefined,

        async fetchSamplePrompts() {
            this.samplePrompts = await (await fetch("sample_prompts/")).json();

            if (this.selectedPrompt === null) {
                this.selectPrompt(this.samplePrompts[0]);
            }
        },

        selectPrompt(prompt) {
            this.selectedPrompt = prompt;
            this.savePromptButtonTitle = "Set Prompt";
        },

        async setNewPrompt(selectedPrompt) {
            let postdata = { fish_prompt: selectedPrompt.function };
            let resp = await fetch("set_prompt/", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(postdata)
            });
            if (resp.ok) {
                // Update attributes of current prompt and select it
                this.samplePrompts[0].demo = selectedPrompt.demo;
                this.samplePrompts[0].right = selectedPrompt.right;
                this.samplePrompts[0].function = selectedPrompt.function;
                this.samplePrompts[0].font_size = selectedPrompt.font_size;
                this.selectedPrompt = this.samplePrompts[0];

                // Note that we set it
                this.savePromptButtonTitle = "Prompt Set!";
            };
        },

        setPrompt() {
            if (this.selectedPrompt) {
                this.setNewPrompt(this.selectedPrompt);
            }
        },

        async init() {
            await this.fetchSamplePrompts();
        },
    }));

    window.Alpine.data("functions", () => ({
        functions: [],
        selectedFunction: null,
        functionDefinition: "",

        async selectFunction(fun) {
            this.selectedFunction = fun;
            await this.fetchFunctionDefinition(this.selectedFunction);
        },

        cleanupFishFunction(contents) {
            /* Replace leading tabs and groups of four spaces at the beginning of a line with two spaces. */
            var lines = contents ? contents.split("\n") : [];
            var rx = /^[\t ]+/;
            for (var i = 0; i < lines.length; i++) {
                var line = lines[i];
                /* Get leading tabs and spaces */
                var whitespace_arr = rx.exec(line);
                if (whitespace_arr) {
                    /* Replace four spaces with two spaces, and tabs with two spaces */
                    var whitespace = whitespace_arr[0];
                    var new_whitespace = whitespace.replace(/(    )|(\t)/g, "  ");
                    lines[i] = new_whitespace + line.slice(whitespace.length);
                }
            }
            return lines.join("\n");
        },

        async fetchFunctionDefinition(name) {
            let resp = await fetch("get_function/", {
                method: "POST",
                headers: { "Content-Type": "application/x-www-form-urlencoded" },
                // TODO are functions guaranteed to newer contain symbols which require escaping?
                body: `what=${name}`,
            });
            this.functionDefinition = (await resp.json())[0];
        },

        async init() {
            this.functions = await (await fetch("functions/")).json();
            this.selectFunction(this.functions[0]);
        },
    }));

    window.Alpine.data("variables", () => ({
        vars: [],
        query: "",
        async init() {
            this.vars = await (await fetch("variables/")).json();
        },
        get filteredVars() {
            return this.vars.filter(
                (v) => v.name.includes(this.query) || v.value.includes(this.query)
            );
        },
    }));

    window.Alpine.data("history", () => ({
        // All history items, as strings.
        allItems: [],
        // How many items per page.
        itemsPerPage: 50,
        // Filtered items grouped into pages. A list of lists. Never empty.
        filteredItems: [],
        // The current page index (0 based).
        currentPage: 0,
        // The filter query.
        query: "",
        // The items which are selected.
        selectedItems: [],

        // Filter all of our items.
        // This could also be a getter though they are not cached
        // and would be recalculated on each page change.
        filterAndGroup() {
            this.filteredItems = this.allItems.filter((item) => item.includes(this.query));
        },

        get currentPageDescription() {
            if (!this.filteredItems.length) {
                return "None";
            }
            return `${this.currentPage + 1} / ${Math.ceil(
                this.filteredItems.length / this.itemsPerPage
            )}`;
        },

        prevPage() {
            this.currentPage = Math.max(this.currentPage - 1, 0);
        },

        nextPage() {
            this.currentPage = Math.min(this.currentPage + 1, this.filteredItems.length - 1);
        },

        selectItem(item) {
            var index = this.selectedItems.indexOf(item);
            if (index >= 0) {
                // Deselect.
                this.selectedItems.splice(index, 1);
            } else {
                this.selectedItems.push(item);
            }
        },

        async deleteHistoryItem(item) {
            await fetch("delete_history_item/", {
                method: "POST",
                headers: { "Content-Type": "application/x-www-form-urlencoded" },
                body: `what=${encodeURIComponent(item)}`,
            });
            let index = this.allItems.indexOf(item);
            this.allItems.splice(index, 1);
            this.filterAndGroup();
        },

        async init() {
            this.allItems = await (await fetch("history/")).json();
            // TODO this should not be necessary as long as we use $watch below
            this.filterAndGroup();

            this.$watch("query", () => {
                this.filterAndGroup();
                this.currentPage = 0;
            });
        },
    }));

    window.Alpine.data("bindings", () => ({
        bindings: [],
        query: "",
        async init() {
            this.bindings = await (await fetch("bindings/")).json();
        },
        get filteredBindings() {
            return this.bindings.filter(
                (b) =>
                    b.command.includes(this.query) ||
                    b.bindings.some((variety) =>
                        variety.readable_binding.toLowerCase().includes(this.query.toLowerCase())
                    )
            );
        },
    }));
});

function color_scheme_respects_color_theme(colorScheme) {
    return colorScheme.by_color_theme.light !== undefined &&
        colorScheme.by_color_theme.dark !== undefined;
}

function default_color_scheme(colorScheme) {
    if (colorScheme.by_color_theme.light !== undefined)
        return colorScheme.by_color_theme.light;
    else if (colorScheme.by_color_theme.dark !== undefined)
        return colorScheme.by_color_theme.dark;
    return colorScheme.by_color_theme.unknown;
}
