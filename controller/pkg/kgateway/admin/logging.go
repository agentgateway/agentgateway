package admin

import (
	"fmt"
	"net/http"
	"sort"
	"strings"

	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
)

// The logging handler allows dynamically changing the log level at runtime.
func addLoggingHandler(path string, mux *http.ServeMux, profiles map[string]dynamicProfileDescription) {
	mux.HandleFunc(path, logging.HTTPLevelHandler)
	profiles[path] = getLoggingDescription
}

// Gets the html/js to display in the UI for the logging endpoint.
func getLoggingDescription() string {
	componentLevels := logging.GetComponentLevels()

	// Build component selector
	var componentSelector strings.Builder
	componentSelector.WriteString(`<select id="componentselector">`)
	// Ensure consistent order for the selector
	components := make([]string, 0, len(componentLevels))
	for comp := range componentLevels {
		components = append(components, comp)
	}
	sort.Strings(components) // Sort alphabetically

	for _, comp := range components {
		componentSelector.WriteString(fmt.Sprintf(`<option value="%s">%s (Current: %s)</option>`, comp, comp, logging.LevelToString(componentLevels[comp])))
	}
	componentSelector.WriteString(`</select>`)

	// Build level selector
	var levelSelector strings.Builder
	levelSelector.WriteString(`<select id="loglevelselector">`)
	supportedLogLevels := []string{"trace", "debug", "info", "warn", "error"}
	for _, level := range supportedLogLevels {
		levelSelector.WriteString(fmt.Sprintf(`<option value="%s">%s</option>`, level, strings.ToUpper(level)))
	}
	levelSelector.WriteString(`</select>`)

	// Display current levels
	var currentLevelsDisplay strings.Builder
	currentLevelsDisplay.WriteString("<h4>Current Levels:</h4><ul>")
	for _, comp := range components {
		currentLevelsDisplay.WriteString(fmt.Sprintf("<li>%s: %s</li>", comp, logging.LevelToString(componentLevels[comp])))
	}
	currentLevelsDisplay.WriteString("</ul><hr/>")

	return currentLevelsDisplay.String() + `Set log level for a specific component.<br/>

Component: ` + componentSelector.String() + `
Level: ` + levelSelector.String() + `

<button onclick="setlevel()">Set Component Level</button>
<button onclick="setAllLevels()">Set All Levels</button>

<script>
function setlevel() {
	var component = document.getElementById('componentselector').value;
	var level = document.getElementById('loglevelselector').value;
	var xhr = new XMLHttpRequest();
	// Use PUT method and construct query parameter as expected by HTTPLevelHandler
	// Assuming the handler is mounted at /logging
	var url = '/logging?' + encodeURIComponent(component) + '=' + encodeURIComponent(level);
	xhr.open('PUT', url, true);
	// No request body needed for PUT with query params for this handler

	xhr.onreadystatechange = function() {
		if (this.readyState == 4) {
			if (this.status == 200) {
				alert("Log level for component '" + component + "' set to: " + level + ". Response: " + this.responseText);
				// Optional: refresh the page or update the display dynamically
				location.reload();
			} else {
				alert("Error setting log level for component '" + component + "'. Status: " + this.status + ". Response: " + this.responseText);
			}
		}
	};

	xhr.send();
}

function setAllLevels() {
	var level = document.getElementById('loglevelselector').value; // Only need the level
	var xhr = new XMLHttpRequest();
	// Construct URL with the global 'level' query parameter
	var url = '/logging?level=' + encodeURIComponent(level);
	xhr.open('PUT', url, true);

	xhr.onreadystatechange = function() {
		if (this.readyState == 4) {
			if (this.status == 200) {
				alert("Log level for ALL components set to: " + level + ". Response: " + this.responseText);
				location.reload(); // Refresh to see updated current levels
			} else {
				alert("Error setting log level for all components. Status: " + this.status + ". Response: " + this.responseText);
			}
		}
	};

	xhr.send();
}
</script>
	`
}
