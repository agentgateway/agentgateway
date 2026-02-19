package main

import (
	"bytes"
	"errors"
	"flag"
	"fmt"
	"go/ast"
	"go/types"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"sort"
	"strings"

	"golang.org/x/tools/go/packages"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-tools/pkg/crd"
	crdmarkers "sigs.k8s.io/controller-tools/pkg/crd/markers"
	"sigs.k8s.io/controller-tools/pkg/loader"
	"sigs.k8s.io/controller-tools/pkg/markers"
	"sigs.k8s.io/yaml"
)

const (
	atLeastOneFieldSetMarker = "kubebuilder:validation:AtLeastOneFieldSet"
	ifThenOnlyFieldsMarker   = "kubebuilder:validation:IfThenOnlyFields"
	controllerGenVersion     = "v0.20.0"
)

type pathList []string

func (p *pathList) String() string {
	return strings.Join(*p, ",")
}

func (p *pathList) Set(value string) error {
	for part := range strings.SplitSeq(value, ",") {
		part = strings.TrimSpace(part)
		if part == "" {
			continue
		}
		*p = append(*p, part)
	}
	return nil
}

// +controllertools:marker:generateHelp:category="CRD validation"
type AtLeastOneFieldSet struct {
	Fields  []string `marker:",optional"`
	Exclude []string `marker:",optional"`
	Message string   `marker:",optional"`
}

// +controllertools:marker:generateHelp:category="CRD validation"
type IfThenOnlyFields struct {
	If      string
	Fields  []string
	Message string `marker:",optional"`
}

func (m AtLeastOneFieldSet) ApplyToSchema(schema *apiextensionsv1.JSONSchemaProps) error {
	allFields := sortedPropertyNames(schema)
	if len(m.Fields) > 0 {
		allFields = dedupeAndSort(append(allFields, m.Fields...))
	}
	if len(m.Exclude) > 0 {
		allFields = dedupeAndSort(append(allFields, m.Exclude...))
	}
	return applyAtLeastOneFieldSet(schema, allFields, m)
}

func (AtLeastOneFieldSet) ApplyPriority() crdmarkers.ApplyPriority {
	return crdmarkers.AtLeastOneOf{}.ApplyPriority() + 1
}

func (m IfThenOnlyFields) ApplyToSchema(schema *apiextensionsv1.JSONSchemaProps) error {
	allFields := sortedPropertyNames(schema)
	if len(m.Fields) > 0 {
		allFields = dedupeAndSort(append(allFields, m.Fields...))
	}
	return applyIfThenOnlyFields(schema, allFields, m)
}

func (IfThenOnlyFields) ApplyPriority() crdmarkers.ApplyPriority {
	return AtLeastOneFieldSet{}.ApplyPriority() + 1
}

func main() {
	var paths pathList
	outputDir := flag.String("output-dir", "", "directory to write CRD YAMLs")
	maxDescLen := flag.Int("max-desc-len", 0, "maximum description length (0 disables descriptions)")
	crdVersion := flag.String("crd-version", apiextensionsv1.SchemeGroupVersion.Version, "CRD API version")
	flag.Var(&paths, "path", "Go package path root to load (repeatable or comma-separated)")
	flag.Parse()

	if *outputDir == "" {
		exitWithErr(fmt.Errorf("--output-dir is required"))
	}
	if len(paths) == 0 {
		exitWithErr(fmt.Errorf("at least one --path is required"))
	}

	if err := os.MkdirAll(*outputDir, 0o755); err != nil {
		exitWithErr(fmt.Errorf("create output dir %q: %w", *outputDir, err))
	}

	if err := generateCRDs(paths, *outputDir, *maxDescLen, *crdVersion); err != nil {
		exitWithErr(err)
	}
}

func exitWithErr(err error) {
	_, _ = fmt.Fprintf(os.Stderr, "custom CRD generation failed: %v\n", err)
	os.Exit(1)
}

func generateCRDs(paths []string, outputDir string, maxDescLen int, crdVersion string) error {
	roots, err := loader.LoadRoots(paths...)
	if err != nil {
		return fmt.Errorf("load roots: %w", err)
	}

	generator := &crd.Generator{}
	parser := &crd.Parser{
		Collector: &markers.Collector{Registry: &markers.Registry{}},
		Checker: &loader.TypeChecker{
			NodeFilters: []loader.NodeFilter{generator.CheckFilter()},
		},
	}

	if err := generator.RegisterMarkers(parser.Collector.Registry); err != nil {
		return fmt.Errorf("register builtin markers: %w", err)
	}
	if err := registerCustomMarkers(parser.Collector.Registry); err != nil {
		return fmt.Errorf("register custom markers: %w", err)
	}

	crd.AddKnownTypes(parser)
	for _, root := range roots {
		parser.NeedPackage(root)
	}

	if err := populateInferredMarkerFields(parser); err != nil {
		return fmt.Errorf("populate inferred marker fields: %w", err)
	}

	metav1Pkg := crd.FindMetav1(roots)
	if metav1Pkg == nil {
		return fmt.Errorf("no objects in roots (nothing imported metav1)")
	}

	kubeKinds := crd.FindKubeKinds(parser, metav1Pkg)
	if len(kubeKinds) == 0 {
		return nil
	}

	for _, groupKind := range kubeKinds {
		parser.NeedCRDFor(groupKind, &maxDescLen)
		crdRaw := parser.CustomResourceDefinitions[groupKind]
		addAttribution(&crdRaw)
		crd.FixTopLevelMetadata(crdRaw)

		converted, err := crd.AsVersion(crdRaw, schema.GroupVersion{Group: apiextensionsv1.SchemeGroupVersion.Group, Version: crdVersion})
		if err != nil {
			return fmt.Errorf("convert CRD %s/%s: %w", groupKind.Group, groupKind.Kind, err)
		}

		crdV1, ok := converted.(*apiextensionsv1.CustomResourceDefinition)
		if !ok {
			return fmt.Errorf("unexpected converted type %T", converted)
		}
		removeDescriptionFromMetadata(crdV1)

		out, err := marshalCRD(crdV1)
		if err != nil {
			return fmt.Errorf("marshal CRD %s/%s: %w", groupKind.Group, groupKind.Kind, err)
		}

		fileName := fmt.Sprintf("%s_%s.yaml", crdRaw.Spec.Group, crdRaw.Spec.Names.Plural)
		filePath := filepath.Join(outputDir, fileName)
		// nolint:gosec // G306: not relevant here
		if err := os.WriteFile(filePath, out, 0o644); err != nil {
			return fmt.Errorf("write CRD %q: %w", filePath, err)
		}
	}

	if loader.PrintErrors(roots, packages.TypeError) {
		return fmt.Errorf("not all generators ran successfully")
	}

	return nil
}

func registerCustomMarkers(registry *markers.Registry) error {
	defs := []*markers.Definition{
		markers.Must(markers.MakeDefinition(atLeastOneFieldSetMarker, markers.DescribesType, AtLeastOneFieldSet{})),
		markers.Must(markers.MakeDefinition(ifThenOnlyFieldsMarker, markers.DescribesType, IfThenOnlyFields{})),
	}
	for _, def := range defs {
		if err := registry.Register(def); err != nil {
			return err
		}
	}
	return nil
}

func populateInferredMarkerFields(parser *crd.Parser) error {
	fieldCache := make(map[crd.TypeIdent][]string)
	inProgress := make(map[crd.TypeIdent]bool)

	for typ, info := range parser.Types {
		needInferredFields := false
		for _, raw := range info.Markers[atLeastOneFieldSetMarker] {
			marker, err := asAtLeastOneFieldSet(raw)
			if err != nil {
				return fmt.Errorf("%s: %w", typ, err)
			}
			if len(marker.Fields) == 0 {
				needInferredFields = true
				break
			}
		}
		if !needInferredFields {
			continue
		}

		inferredFields, err := allJSONFieldNamesForType(parser, typ, info, fieldCache, inProgress)
		if err != nil {
			return fmt.Errorf("%s: %w", typ, err)
		}

		if markerVals, ok := info.Markers[atLeastOneFieldSetMarker]; ok {
			for i, raw := range markerVals {
				marker, err := asAtLeastOneFieldSet(raw)
				if err != nil {
					return fmt.Errorf("%s: %w", typ, err)
				}
				if len(marker.Fields) == 0 {
					marker.Fields = append([]string(nil), inferredFields...)
					markerVals[i] = marker
				}
			}
			info.Markers[atLeastOneFieldSetMarker] = markerVals
		}
	}

	return nil
}

func asAtLeastOneFieldSet(raw any) (AtLeastOneFieldSet, error) {
	switch marker := raw.(type) {
	case AtLeastOneFieldSet:
		return marker, nil
	case *AtLeastOneFieldSet:
		if marker == nil {
			return AtLeastOneFieldSet{}, fmt.Errorf("unexpected nil marker for %s", atLeastOneFieldSetMarker)
		}
		return *marker, nil
	default:
		return AtLeastOneFieldSet{}, fmt.Errorf("unexpected marker value %T for %s", raw, atLeastOneFieldSetMarker)
	}
}

func applyAtLeastOneFieldSet(schema *apiextensionsv1.JSONSchemaProps, allFields []string, marker AtLeastOneFieldSet) error {
	fields, err := resolveFields(allFields, marker.Fields, marker.Exclude)
	if err != nil {
		return err
	}
	if len(fields) == 0 {
		return errors.New("AtLeastOneFieldSet resolved to zero fields")
	}

	message := marker.Message
	if message == "" {
		message = fmt.Sprintf("at least one of the fields in %v must be set", fields)
	}

	return crdmarkers.XValidation{
		Rule:    fmt.Sprintf("%s >= 1", fieldsToOneOfCelRuleStr(fields)),
		Message: message,
	}.ApplyToSchema(schema)
}

func applyIfThenOnlyFields(schema *apiextensionsv1.JSONSchemaProps, allFields []string, marker IfThenOnlyFields) error {
	if strings.TrimSpace(marker.If) == "" {
		return errors.New("IfThenOnlyFields requires 'if'")
	}
	if len(marker.Fields) == 0 {
		return errors.New("IfThenOnlyFields requires at least one field")
	}

	allowedFields, err := validateFieldList(allFields, dedupeAndSort(marker.Fields), "fields")
	if err != nil {
		return err
	}

	allowed := make(map[string]struct{}, len(allowedFields))
	for _, field := range allowedFields {
		allowed[field] = struct{}{}
	}

	disallowedFields := make([]string, 0, len(allFields))
	for _, field := range allFields {
		if _, ok := allowed[field]; !ok {
			disallowedFields = append(disallowedFields, field)
		}
	}
	if len(disallowedFields) == 0 {
		return nil
	}

	message := marker.Message
	if message == "" {
		message = fmt.Sprintf("only fields in %v may be set when %s", allowedFields, marker.If)
	}

	return crdmarkers.XValidation{
		Rule:    fmt.Sprintf("%s ? %s == 0 : true", marker.If, fieldsToOneOfCelRuleStr(disallowedFields)),
		Message: message,
	}.ApplyToSchema(schema)
}

func resolveFields(allFields, includes, excludes []string) ([]string, error) {
	var err error
	includes, err = normalizeFieldList(includes, "fields")
	if err != nil {
		return nil, err
	}
	excludes, err = normalizeFieldList(excludes, "exclude")
	if err != nil {
		return nil, err
	}

	if len(includes) > 0 {
		includes, err = validateFieldList(allFields, includes, "fields")
		if err != nil {
			return nil, err
		}
	}

	if len(excludes) > 0 {
		excludes, err = validateFieldList(allFields, excludes, "exclude")
		if err != nil {
			return nil, err
		}
	}

	if len(includes) == 0 {
		includes = append([]string(nil), allFields...)
	}

	excluded := make(map[string]struct{}, len(excludes))
	for _, field := range excludes {
		excluded[field] = struct{}{}
	}

	result := make([]string, 0, len(includes))
	for _, field := range includes {
		if _, ok := excluded[field]; !ok {
			result = append(result, field)
		}
	}

	return result, nil
}

func normalizeFieldList(fields []string, listName string) ([]string, error) {
	fields = dedupeAndSort(fields)
	for _, field := range fields {
		if strings.Contains(field, ".") {
			return nil, fmt.Errorf("%s: cannot reference nested fields: %s", listName, strings.Join(fields, ","))
		}
	}
	return fields, nil
}

func validateFieldList(allFields []string, fields []string, listName string) ([]string, error) {
	fields, err := normalizeFieldList(fields, listName)
	if err != nil {
		return nil, err
	}
	if len(fields) == 0 {
		return fields, nil
	}

	all := make(map[string]struct{}, len(allFields))
	for _, name := range allFields {
		all[name] = struct{}{}
	}

	for _, field := range fields {
		if _, ok := all[field]; !ok {
			return nil, fmt.Errorf("%s: unknown field %q", listName, field)
		}
	}

	return fields, nil
}

func dedupeAndSort(in []string) []string {
	if len(in) == 0 {
		return nil
	}

	seen := make(map[string]struct{}, len(in))
	out := make([]string, 0, len(in))
	for _, v := range in {
		v = strings.TrimSpace(v)
		if v == "" {
			continue
		}
		if _, ok := seen[v]; ok {
			continue
		}
		seen[v] = struct{}{}
		out = append(out, v)
	}
	sort.Strings(out)
	return out
}

func sortedPropertyNames(schema *apiextensionsv1.JSONSchemaProps) []string {
	namesSet := map[string]struct{}{}
	for name := range schema.Properties {
		namesSet[name] = struct{}{}
	}

	var collect func(items []apiextensionsv1.JSONSchemaProps)
	collect = func(items []apiextensionsv1.JSONSchemaProps) {
		for _, item := range items {
			for name := range item.Properties {
				namesSet[name] = struct{}{}
			}
			if len(item.AllOf) > 0 {
				collect(item.AllOf)
			}
		}
	}
	collect(schema.AllOf)

	names := make([]string, 0, len(namesSet))
	for name := range namesSet {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}

func allJSONFieldNamesForType(
	parser *crd.Parser,
	typ crd.TypeIdent,
	info *markers.TypeInfo,
	fieldCache map[crd.TypeIdent][]string,
	inProgress map[crd.TypeIdent]bool,
) ([]string, error) {
	if cached, ok := fieldCache[typ]; ok {
		return append([]string(nil), cached...), nil
	}

	if inProgress[typ] {
		return nil, fmt.Errorf("%s: recursive inline embedding detected", typ)
	}

	if _, isStruct := info.RawSpec.Type.(*ast.StructType); !isStruct {
		return nil, fmt.Errorf("%s marker is only supported on struct types", typ)
	}

	inProgress[typ] = true
	defer delete(inProgress, typ)

	fieldSet := map[string]struct{}{}
	for _, field := range info.Fields {
		fieldName, inline, include := jsonFieldInfo(field.Tag)
		if !include {
			continue
		}

		if inline {
			embeddedType, ok := typeIdentForExpr(parser, typ.Package, field.RawField.Type)
			if !ok {
				continue
			}

			embeddedInfo := parser.LookupType(embeddedType.Package, embeddedType.Name)
			if embeddedInfo == nil {
				continue
			}

			embeddedFields, err := allJSONFieldNamesForType(parser, embeddedType, embeddedInfo, fieldCache, inProgress)
			if err != nil {
				return nil, err
			}
			for _, embeddedField := range embeddedFields {
				fieldSet[embeddedField] = struct{}{}
			}
			continue
		}

		fieldSet[fieldName] = struct{}{}
	}

	fields := make([]string, 0, len(fieldSet))
	for field := range fieldSet {
		fields = append(fields, field)
	}
	sort.Strings(fields)

	fieldCache[typ] = fields
	return append([]string(nil), fields...), nil
}

func jsonFieldInfo(tag reflect.StructTag) (name string, inline bool, include bool) {
	jsonTag, hasTag := tag.Lookup("json")
	if !hasTag {
		return "", false, false
	}

	jsonOpts := strings.Split(jsonTag, ",")
	if len(jsonOpts) == 1 && jsonOpts[0] == "-" {
		return "", false, false
	}

	if slices.Contains(jsonOpts[1:], "inline") {
		inline = true
	}

	name = jsonOpts[0]
	inline = inline || name == ""
	if inline {
		return "", true, true
	}

	return name, false, true
}

func typeIdentForExpr(parser *crd.Parser, contextPkg *loader.Package, expr ast.Expr) (crd.TypeIdent, bool) {
	if contextPkg == nil {
		return crd.TypeIdent{}, false
	}

	contextPkg.NeedTypesInfo()
	goType := contextPkg.TypesInfo.TypeOf(expr)
	if goType == nil {
		return crd.TypeIdent{}, false
	}

	for {
		goType = types.Unalias(goType)

		switch typed := goType.(type) {
		case *types.Pointer:
			goType = typed.Elem()
		case *types.Named:
			obj := typed.Obj()
			if obj == nil || obj.Pkg() == nil {
				return crd.TypeIdent{}, false
			}

			typePkg := packageForPath(parser, contextPkg, obj.Pkg().Path())
			if typePkg == nil {
				return crd.TypeIdent{}, false
			}

			return crd.TypeIdent{
				Package: typePkg,
				Name:    obj.Name(),
			}, true
		default:
			return crd.TypeIdent{}, false
		}
	}
}

func packageForPath(parser *crd.Parser, contextPkg *loader.Package, pkgPath string) *loader.Package {
	targetPath := loader.NonVendorPath(pkgPath)
	if loader.NonVendorPath(contextPkg.PkgPath) == targetPath {
		return contextPkg
	}

	if imported := contextPkg.Imports()[targetPath]; imported != nil {
		return imported
	}

	for _, imported := range contextPkg.Imports() {
		if loader.NonVendorPath(imported.PkgPath) == targetPath {
			return imported
		}
	}

	for typ := range parser.Types {
		if loader.NonVendorPath(typ.Package.PkgPath) == targetPath {
			return typ.Package
		}
	}

	return nil
}

func fieldsToOneOfCelRuleStr(fields []string) string {
	var builder strings.Builder
	builder.WriteString("[")
	for i, field := range fields {
		if i > 0 {
			builder.WriteString(",")
		}
		builder.WriteString("has(self.")
		builder.WriteString(field)
		builder.WriteString(")")
	}
	builder.WriteString("].filter(x,x==true).size()")
	return builder.String()
}

func addAttribution(crd *apiextensionsv1.CustomResourceDefinition) {
	if crd.ObjectMeta.Annotations == nil {
		crd.ObjectMeta.Annotations = map[string]string{}
	}
	crd.ObjectMeta.Annotations["controller-gen.kubebuilder.io/version"] = controllerGenVersion
}

func removeDescriptionFromMetadata(crd *apiextensionsv1.CustomResourceDefinition) {
	for _, versionSpec := range crd.Spec.Versions {
		if versionSpec.Schema != nil {
			removeDescriptionFromMetadataProps(versionSpec.Schema.OpenAPIV3Schema)
		}
	}
}

func removeDescriptionFromMetadataProps(schema *apiextensionsv1.JSONSchemaProps) {
	metadataSchema, ok := schema.Properties["metadata"]
	if !ok {
		return
	}
	if metadataSchema.Description == "" {
		return
	}
	metadataSchema.Description = ""
	schema.Properties["metadata"] = metadataSchema
}

func marshalCRD(crd *apiextensionsv1.CustomResourceDefinition) ([]byte, error) {
	type output struct {
		APIVersion string                                       `json:"apiVersion"`
		Kind       string                                       `json:"kind"`
		Metadata   map[string]any                               `json:"metadata,omitempty"`
		Spec       apiextensionsv1.CustomResourceDefinitionSpec `json:"spec"`
	}

	metadataBytes, err := yaml.Marshal(crd.ObjectMeta)
	if err != nil {
		return nil, err
	}

	metadata := map[string]any{}
	if err := yaml.Unmarshal(metadataBytes, &metadata); err != nil {
		return nil, err
	}
	delete(metadata, "creationTimestamp")

	out, err := yaml.Marshal(output{
		APIVersion: crd.APIVersion,
		Kind:       crd.Kind,
		Metadata:   metadata,
		Spec:       crd.Spec,
	})
	if err != nil {
		return nil, err
	}

	if bytes.HasPrefix(out, []byte("---\n")) {
		return out, nil
	}

	return append([]byte("---\n"), out...), nil
}
