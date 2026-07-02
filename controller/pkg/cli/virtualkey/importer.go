package virtualkey

import (
	"encoding/csv"
	"encoding/json"
	"fmt"
	"io"
	"maps"
	"os"
	"sort"
	"strings"
)

const virtualKeysLabel = "agentgateway.dev/virtual-keys"
const defaultMaxSerializedSecretSize = 1024 * 1024
const maxSecretNameLength = 253

type importOptions struct {
	SecretName              string
	Namespace               string
	Labels                  map[string]string
	Strict                  bool
	EscapeIDs               bool
	MaxSerializedSecretSize int
}

type importResult struct {
	Secrets       []secretManifest
	Secret        secretManifest
	Warnings      []string
	GeneratedRows []int
	ImportedKeys  map[int]string
}

type secretManifest struct {
	APIVersion string            `json:"apiVersion" yaml:"apiVersion"`
	Kind       string            `json:"kind" yaml:"kind"`
	Metadata   manifestMetadata  `json:"metadata" yaml:"metadata"`
	Type       string            `json:"type" yaml:"type"`
	StringData map[string]string `json:"stringData" yaml:"stringData"`
}

type manifestMetadata struct {
	Name      string            `json:"name" yaml:"name"`
	Namespace string            `json:"namespace,omitempty" yaml:"namespace,omitempty"`
	Labels    map[string]string `json:"labels,omitempty" yaml:"labels,omitempty"`
}

type manifestList struct {
	APIVersion string           `json:"apiVersion" yaml:"apiVersion"`
	Kind       string           `json:"kind" yaml:"kind"`
	Items      []secretManifest `json:"items" yaml:"items"`
}

type csvRow struct {
	Line     int
	Columns  map[string]string
	Metadata map[string]string
}

type secretEntry struct {
	Name  string
	Value string
	Line  int
}

func importCSV(r io.Reader, opts importOptions) (*importResult, error) {
	if opts.SecretName == "" {
		return nil, fmt.Errorf("--secret-name is required")
	}
	if opts.MaxSerializedSecretSize == 0 {
		opts.MaxSerializedSecretSize = defaultMaxSerializedSecretSize
	}
	if opts.MaxSerializedSecretSize < 0 {
		return nil, fmt.Errorf("max serialized Secret size must be greater than 0")
	}
	reader := csv.NewReader(r)
	reader.FieldsPerRecord = -1
	reader.TrimLeadingSpace = true

	header, err := reader.Read()
	if err != nil {
		if err == io.EOF {
			return nil, fmt.Errorf("CSV input is empty")
		}
		return nil, fmt.Errorf("read CSV header: %w", err)
	}
	headers, err := normalizeHeaders(header)
	if err != nil {
		return nil, err
	}

	result := &importResult{
		ImportedKeys: map[int]string{},
	}

	seenEntries := map[string]int{}
	seenIDs := map[string][]int{}
	seenKeys := map[string][]int{}
	var entries []secretEntry
	line := 1
	for {
		record, err := reader.Read()
		if err == io.EOF {
			break
		}
		line++
		if err != nil {
			return nil, fmt.Errorf("read CSV row %d: %w", line, err)
		}
		if isEmptyRecord(record) {
			continue
		}
		row, err := parseCSVRow(line, headers, record)
		if err != nil {
			return nil, err
		}
		entryName, encoded, key, generated, err := buildEntry(row, opts.EscapeIDs)
		if err != nil {
			return nil, err
		}
		if previous, ok := seenEntries[entryName]; ok {
			return nil, fmt.Errorf("duplicate Secret entry %q at rows %d and %d", entryName, previous, row.Line)
		}
		seenEntries[entryName] = row.Line
		entries = append(entries, secretEntry{Name: entryName, Value: encoded, Line: row.Line})
		result.ImportedKeys[row.Line] = key
		seenKeys[key] = append(seenKeys[key], row.Line)
		if generated {
			result.GeneratedRows = append(result.GeneratedRows, row.Line)
		}
		if id := row.Metadata["id"]; id != "" {
			seenIDs[id] = append(seenIDs[id], row.Line)
		} else {
			result.Warnings = append(result.Warnings, fmt.Sprintf("row %d has no id metadata; it will authenticate but has no stable virtual-key identity", row.Line))
		}
	}
	if len(entries) == 0 {
		return nil, fmt.Errorf("CSV input contains no data rows")
	}

	secrets, err := splitSecrets(entries, opts)
	if err != nil {
		return nil, err
	}
	result.Secrets = secrets
	result.Secret = secrets[0]

	for _, rows := range sortedDuplicateRows(seenKeys) {
		result.Warnings = append(result.Warnings, fmt.Sprintf("duplicate API key value at rows %s; runtime deduplicates matching raw keys", formatRows(rows)))
	}
	for _, rows := range sortedDuplicateRows(seenIDs) {
		result.Warnings = append(result.Warnings, fmt.Sprintf("duplicate id metadata at rows %s; runtime keeps the first sorted key", formatRows(rows)))
	}
	if opts.Strict && len(result.Warnings) > 0 {
		return nil, fmt.Errorf("strict validation failed: %s", strings.Join(result.Warnings, "; "))
	}
	return result, nil
}

func (r *importResult) Manifest() any {
	if len(r.Secrets) == 1 {
		return r.Secrets[0]
	}
	return manifestList{
		APIVersion: "v1",
		Kind:       "List",
		Items:      r.Secrets,
	}
}

func splitSecrets(entries []secretEntry, opts importOptions) ([]secretManifest, error) {
	var secrets []secretManifest
	current := newSecretManifest(opts.SecretName, opts.Namespace, opts.Labels)
	for _, entry := range entries {
		candidate := cloneSecretManifest(current)
		candidate.StringData[entry.Name] = entry.Value
		size, err := serializedSecretSize(candidate)
		if err != nil {
			return nil, fmt.Errorf("measure Secret %q: %w", candidate.Metadata.Name, err)
		}
		if size <= opts.MaxSerializedSecretSize {
			current = candidate
			continue
		}
		if len(current.StringData) == 0 {
			return nil, fmt.Errorf("row %d Secret entry %q exceeds max serialized Secret size %d bytes", entry.Line, entry.Name, opts.MaxSerializedSecretSize)
		}
		secrets = append(secrets, current)
		name, err := suffixedSecretName(opts.SecretName, len(secrets)+1)
		if err != nil {
			return nil, err
		}
		current = newSecretManifest(name, opts.Namespace, opts.Labels)
		current.StringData[entry.Name] = entry.Value
		size, err = serializedSecretSize(current)
		if err != nil {
			return nil, fmt.Errorf("measure Secret %q: %w", current.Metadata.Name, err)
		}
		if size > opts.MaxSerializedSecretSize {
			return nil, fmt.Errorf("row %d Secret entry %q exceeds max serialized Secret size %d bytes", entry.Line, entry.Name, opts.MaxSerializedSecretSize)
		}
	}
	if len(current.StringData) > 0 {
		secrets = append(secrets, current)
	}
	return secrets, nil
}

func newSecretManifest(name, namespace string, labels map[string]string) secretManifest {
	return secretManifest{
		APIVersion: "v1",
		Kind:       "Secret",
		Metadata: manifestMetadata{
			Name:      name,
			Namespace: namespace,
			Labels:    secretLabels(labels),
		},
		Type:       "Opaque",
		StringData: map[string]string{},
	}
}

func cloneSecretManifest(secret secretManifest) secretManifest {
	clone := secret
	clone.Metadata.Labels = map[string]string{}
	maps.Copy(clone.Metadata.Labels, secret.Metadata.Labels)
	clone.StringData = map[string]string{}
	maps.Copy(clone.StringData, secret.StringData)
	return clone
}

func serializedSecretSize(secret secretManifest) (int, error) {
	b, err := json.Marshal(secret)
	if err != nil {
		return 0, err
	}
	return len(b), nil
}

func suffixedSecretName(base string, ordinal int) (string, error) {
	if ordinal == 1 {
		return base, nil
	}
	name := fmt.Sprintf("%s-%04d", base, ordinal)
	if len(name) > maxSecretNameLength {
		return "", fmt.Errorf("secret name %q is too long to split into Secret %d; suffixed name %q exceeds %d characters", base, ordinal, name, maxSecretNameLength)
	}
	return name, nil
}

func normalizeHeaders(header []string) ([]string, error) {
	seen := map[string]struct{}{}
	headers := make([]string, len(header))
	for i, h := range header {
		name := strings.TrimSpace(h)
		if name == "" {
			return nil, fmt.Errorf("CSV header column %d is empty", i+1)
		}
		if _, ok := seen[name]; ok {
			return nil, fmt.Errorf("duplicate CSV header %q", name)
		}
		seen[name] = struct{}{}
		headers[i] = name
	}
	return headers, nil
}

func parseCSVRow(line int, headers, record []string) (csvRow, error) {
	row := csvRow{
		Line:     line,
		Columns:  map[string]string{},
		Metadata: map[string]string{},
	}
	for i, header := range headers {
		value := ""
		if i < len(record) {
			value = record[i]
		}
		switch {
		case header == "key":
			row.Columns[header] = strings.TrimSpace(value)
		case isReservedColumn(header):
			row.Columns[header] = strings.TrimSpace(value)
		case strings.HasPrefix(header, "metadata."):
			name := strings.TrimPrefix(header, "metadata.")
			if name == "" {
				return row, fmt.Errorf("row %d column %q has an empty metadata key", line, header)
			}
			row.Metadata[name] = strings.TrimSpace(value)
		default:
			row.Metadata[header] = strings.TrimSpace(value)
		}
	}
	if id := row.Columns["id"]; id != "" {
		if metadataID := row.Metadata["id"]; metadataID != "" && metadataID != id {
			return row, fmt.Errorf("row %d has conflicting id and metadata.id values", line)
		}
		row.Metadata["id"] = id
	}
	return row, nil
}

func buildEntry(row csvRow, escapeIDs bool) (string, string, string, bool, error) {
	id := row.Metadata["id"]
	if id != "" {
		normalized, err := normalizeID(id, escapeIDs)
		if err != nil {
			return "", "", "", false, fmt.Errorf("row %d column id: %w", row.Line, err)
		}
		row.Metadata["id"] = normalized
		id = normalized
	}

	entryName := row.Columns["entry"]
	if entryName == "" {
		if id != "" {
			entryName = "vk-" + sanitizeLabel(id)
		} else {
			entryName = fmt.Sprintf("row-%06d", row.Line)
		}
	} else {
		entryName = sanitizeLabel(entryName)
	}
	if entryName == "" {
		return "", "", "", false, fmt.Errorf("row %d has an empty Secret entry name", row.Line)
	}

	key := row.Columns["key"]
	generated := false
	if key == "" {
		label := row.Columns["entry"]
		if label == "" {
			label = id
		}
		if label == "" {
			label = fmt.Sprintf("row-%06d", row.Line)
		}
		var err error
		key, err = generateAPIKey(label)
		if err != nil {
			return "", "", "", false, fmt.Errorf("row %d generated key: %w", row.Line, err)
		}
		generated = true
	}

	metadata := map[string]string{}
	for k, v := range row.Metadata {
		if strings.TrimSpace(k) == "" {
			return "", "", "", false, fmt.Errorf("row %d has an empty metadata key", row.Line)
		}
		if v == "" {
			continue
		}
		metadata[k] = v
	}
	entry := apiKeyEntry{Key: key, Metadata: metadata}
	encoded, err := json.Marshal(entry)
	if err != nil {
		return "", "", "", false, fmt.Errorf("row %d encode Secret entry: %w", row.Line, err)
	}
	return entryName, string(encoded), key, generated, nil
}

func normalizeID(value string, escape bool) (string, error) {
	if escape {
		return escapeID(value), nil
	}
	if value == "<missing>" {
		return "", fmt.Errorf("value %q is reserved; use --escape-special-characters to store an escaped value", value)
	}
	if strings.ContainsAny(value, "|^`") {
		return "", fmt.Errorf("value contains reserved special characters; use --escape-special-characters to store an escaped value")
	}
	return value, nil
}

func escapeID(value string) string {
	if value == "<missing>" {
		return `\<missing>`
	}
	var b strings.Builder
	for _, r := range value {
		switch r {
		case '\\':
			b.WriteString(`\\`)
		case '|':
			b.WriteString(`\|`)
		case '^':
			b.WriteString(`\^`)
		case '`':
			b.WriteString("\\`")
		default:
			b.WriteRune(r)
		}
	}
	return b.String()
}

func secretLabels(extra map[string]string) map[string]string {
	labels := map[string]string{}
	maps.Copy(labels, extra)
	labels[virtualKeysLabel] = "true"
	return labels
}

func parseLabels(values []string) (map[string]string, error) {
	labels := map[string]string{}
	for _, value := range values {
		key, val, ok := strings.Cut(value, "=")
		if !ok || strings.TrimSpace(key) == "" {
			return nil, fmt.Errorf("label %q must be in key=value form", value)
		}
		labels[strings.TrimSpace(key)] = strings.TrimSpace(val)
	}
	return labels, nil
}

func sortedDuplicateRows(values map[string][]int) [][]int {
	var duplicates [][]int
	for _, rows := range values {
		if len(rows) > 1 {
			sort.Ints(rows)
			duplicates = append(duplicates, rows)
		}
	}
	sort.Slice(duplicates, func(i, j int) bool {
		return duplicates[i][0] < duplicates[j][0]
	})
	return duplicates
}

func formatRows(rows []int) string {
	parts := make([]string, len(rows))
	for i, row := range rows {
		parts[i] = fmt.Sprint(row)
	}
	return strings.Join(parts, ", ")
}

func isReservedColumn(header string) bool {
	switch header {
	case "entry", "id", "secretName", "namespace":
		return true
	default:
		return false
	}
}

func isEmptyRecord(record []string) bool {
	for _, field := range record {
		if strings.TrimSpace(field) != "" {
			return false
		}
	}
	return true
}

func warnIfPermissive(path string) string {
	if path == "-" {
		return ""
	}
	info, err := os.Stat(path)
	if err != nil {
		return ""
	}
	if info.Mode().Perm()&0o077 != 0 {
		return "input file is readable by other users; consider chmod 0600"
	}
	return ""
}
