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
        selectedColorSetting: false,
        // Every colour variable the shell declares, in declaration order.
        colourVariables: window.__fish_colour_variables,
        // The variable a scope path stands for, so the classified sample can name it.
        variableOfPath: Object.fromEntries(
            window.__fish_colour_variables
                .filter((variable) => variable.path)
                .map((variable) => [variable.path, variable.variable])),
        // The declaration's entry for a variable, which states the variables it resolves
        // through.
        colourVariableOf: Object.fromEntries(
            window.__fish_colour_variables
                .map((variable) => [variable.variable, variable])),
        // The attribute a variable carries.
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
        // Which field of the selected variable the colour grid writes.
        editingField: "foreground",

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
            this.$nextTick(() => {
                if (this.$refs.specificity) this.$refs.specificity.focus();
            });
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
        },

        // Whether the stop's own variable names anything: an unset one is what it
        // inherits, and is drawn dimmer so that the rail shows which overrides exist.
        stopIsSet(stop) {
            return !!this.currentColorScheme().colordata[stop.variable];
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
            this.beginCustomizationWithSetting(variable);
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
        // enclosing scope, and every run for the root.  A hovered variable takes over the
        // mark while the pointer is on it.
        pieceInActiveStop(pieceIndex) {
            const piece = this.colourSample[pieceIndex];
            if (this.hoveredVariable) {
                return this.resolutionOf(piece).includes(this.hoveredVariable);
            }
            const stop = this.pickedStops[this.hoveredStopIndex ?? this.selectedStopIndex];
            if (!stop) return false;
            switch (stop.kind) {
                case "leaf":
                    // A run picked in the sample is one run; a variable picked from the
                    // list is every run the pass gave that scope.
                    return this.pickedPieceIndex !== null
                        ? pieceIndex === this.pickedPieceIndex
                        : this.variableForPiece(piece) === stop.variable;
                case "root":
                    return true;
                case "enclosing":
                    return piece.enclosing.includes(stop.path);
                case "chain":
                    return this.resolutionOf(piece).includes(stop.variable);
            }
            return false;
        },

        // The style a stop's own variable draws: its value, else the chain it resolves
        // through, else the root.  A stop whose variable names nothing is drawn by what it
        // inherits, which is what it would change.
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

        // The specification the picker writes: the variable's own value, or a value that
        // names nothing yet, which is what setting an override starts from.  A scope the
        // theme says nothing about writes nothing until a field is set, so the sample draws
        // it by what it inherits either way.
        writtenSpecification() {
            const variable = this.selectedColorSetting;
            if (!variable) return undefined;
            const colourScheme = this.currentColorScheme();
            if (!colourScheme.colordata[variable]) {
                colourScheme.colordata[variable] = {
                    name: variable,
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

        fieldValueStyle() {
            const colour = this.selectedFieldColour();
            return {
                "background-color": colour ? interpret_color(colour) : this.previewBackgroundColour(),
            };
        },

        fieldValueTitle() {
            const colour = this.selectedFieldColour();
            return colour ? colour : "no value yet";
        },

        selectField(field) {
            this.editingField = field;
        },

        selectedFlag(flag) {
            const specification = this.selectedSpecification();
            return !!(specification && specification[flag]);
        },

        // Switch one flag of the selected variable on or off.  The flags are one field of
        // a `set_color` value, so the value states all of them; turning the underline on
        // makes the grid write the underline colour.
        toggleFlag(flag) {
            const specification = this.writtenSpecification();
            if (!specification) return;
            if (flag === "underline") {
                specification.underline = specification.underline ? null : "single";
                if (specification.underline) this.editingField = "underline_colour";
            } else {
                specification[flag] = !specification[flag];
            }
            this.noteThemeChanged();
        },

        // The page's own text colour, which a variable that names no foreground inherits.
        defaultTextColour() {
            return "#" + text_color_for_color(this.actualTerminalBackgroundColor);
        },

        // The controls sit on the background the sample preview draws on, so that they read
        // as part of the preview rather than as swatches of their own.
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
            // The glyph shows the underline's own color when it names one, so that the
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

        flagButtonStyle(flag) {
            const style = this.effectiveStyleOf(this.selectedColorSetting);
            const css = {
                "color": style.color || this.defaultTextColour(),
                "background-color": this.previewBackgroundColour(),
            };
            if (flag === "bold") css["font-weight"] = "bold";
            if (flag === "italics") css["font-style"] = "italic";
            if (flag === "dim") css["opacity"] = "0.6";
            return css;
        },

        // The style a piece of the sample resolves to, composed as the shell composes it:
        // the root, the enclosing scopes from the outermost, the leaf's chain from its
        // least specific step, the leaf, and then the attributes it carries.  Each
        // variable writes the fields it names, a more specific one after a less specific
        // one, and `--reset` clears the fields before it writes.
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

        // The fields a variable's value names.
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

        // One attribute's fields written over the style: a merge writes what it names, and
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

        // The CSS a composed style draws.
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

        // What the picker calls a setting: a declared variable states itself, and the
        // variables the declaration does not hold — the prompt's own — are named by their
        // spelling.
        titleForSetting(name) {
            const variable = this.colourVariables.find(
                (variable) => variable.variable === name);
            return variable ? variable.description : user_visible_title_for_setting_name(name);
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
            this.selectedColorSetting = false;
            this.customizationActive = false;
            this.csEditingType = false;
            this.clearPicked();
            //TODO: Save button should be shown only when colors are changed
            this.showSaveButton = true;

            this.noteThemeChanged();
        },

        text_color_for_color: text_color_for_color,

        border_color_for_color: border_color_for_color,

        interpret_color: interpret_color,

        get colorArraysArray() {
            var result = null;
            if (this.currentColorScheme().colors &&
                this.currentColorScheme().colors.length > 0)
                result = get_colors_as_nested_array(this.currentColorScheme().colors, 32);
            else result = get_colors_as_nested_array(term_256_colors, 32);
            return result;
        },

        customizationActive: false,
        csEditingType: false,
        beginCustomizationWithSetting(setting) {
            if (!this.customizationActive) {
                this.customizationActive = true;
                this.selectedColorSetting = setting;
                this.csEditingType = setting;
                this.csUserVisibleTitle = this.titleForSetting(setting);
            }
        },

        selectColorSetting(name) {
            this.selectedColorSetting = name;
            this.csEditingType = this.customizationActive ? name : "";
            this.csUserVisibleTitle = this.titleForSetting(name);
            this.beginCustomizationWithSetting(name);
        },

        toggleCustomizationActive() {
            if (!this.customizationActive) {
                this.beginCustomizationWithSetting(this.selectedColorSetting || "fish_color_command");
            } else {
                this.customizationActive = false;
                this.selectedColorSetting = "";
                this.csEditingType = "";
            }
        },

        changeSelectedColor(color) {
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
                    break;
            }
            this.currentColorScheme().colors[this.selectedColorSetting] = specification.color;
            this.noteThemeChanged();
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

        noteThemeChanged() {
            this.saveThemeButtonTitle = "Set Theme";
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
            if (resp.ok) {
                this.saveThemeButtonTitle = "Theme Set!";
            }
        },

        async init() {
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
