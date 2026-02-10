package kraalzibar

import (
	"context"
	"io"

	pb "github.com/pubkraal/kraalzibar/sdks/go/proto/kraalzibar/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
)

// Client is the Kraalzibar gRPC client.
type Client struct {
	conn          *grpc.ClientConn
	permissions   pb.PermissionServiceClient
	relationships pb.RelationshipServiceClient
	schemas       pb.SchemaServiceClient
	config        clientConfig
}

// NewClient creates a new Kraalzibar client connected to the given target.
func NewClient(target string, opts ...ClientOption) (*Client, error) {
	cfg := defaultConfig()
	for _, o := range opts {
		o(&cfg)
	}

	dialOpts := cfg.dialOptions
	if cfg.insecure {
		dialOpts = append(dialOpts, grpc.WithTransportCredentials(insecure.NewCredentials()))
	}
	if cfg.apiKey != "" {
		dialOpts = append(dialOpts, grpc.WithUnaryInterceptor(apiKeyInterceptor(cfg.apiKey)))
		dialOpts = append(dialOpts, grpc.WithStreamInterceptor(apiKeyStreamInterceptor(cfg.apiKey)))
	}

	conn, err := grpc.NewClient(target, dialOpts...)
	if err != nil {
		return nil, &Error{Code: CodeUnavailable, Message: err.Error()}
	}

	return &Client{
		conn:          conn,
		permissions:   pb.NewPermissionServiceClient(conn),
		relationships: pb.NewRelationshipServiceClient(conn),
		schemas:       pb.NewSchemaServiceClient(conn),
		config:        cfg,
	}, nil
}

// Close closes the underlying gRPC connection.
func (c *Client) Close() error {
	return c.conn.Close()
}

func (c *Client) ctx(parent context.Context) (context.Context, context.CancelFunc) {
	return context.WithTimeout(parent, c.config.timeout)
}

// CheckPermissionRequest is the input for CheckPermission.
type CheckPermissionRequest struct {
	Resource   ObjectRef
	Permission string
	Subject    SubjectRef
}

// CheckPermissionResponse is the result of CheckPermission.
type CheckPermissionResponse struct {
	Allowed bool
}

// CheckPermission checks whether a subject has a permission on a resource.
func (c *Client) CheckPermission(ctx context.Context, req CheckPermissionRequest) (*CheckPermissionResponse, error) {
	ctx, cancel := c.ctx(ctx)
	defer cancel()

	resp, err := c.permissions.CheckPermission(ctx, &pb.CheckPermissionRequest{
		Consistency: fullConsistency(),
		Resource:    objectRefToProto(req.Resource),
		Permission:  req.Permission,
		Subject:     subjectRefToProto(req.Subject),
	})
	if err != nil {
		return nil, errorFromStatus(err)
	}

	allowed := resp.Permissionship == pb.CheckPermissionResponse_PERMISSIONSHIP_HAS_PERMISSION
	return &CheckPermissionResponse{Allowed: allowed}, nil
}

// WriteRelationshipsRequest is the input for WriteRelationships.
type WriteRelationshipsRequest struct {
	Updates []RelationshipUpdate
}

// RelationshipUpdate describes a single relationship write or delete.
type RelationshipUpdate struct {
	Operation RelationshipOperation
	Object    ObjectRef
	Relation  string
	Subject   SubjectRef
}

// RelationshipOperation is either Touch or Delete.
type RelationshipOperation int

const (
	OperationTouch  RelationshipOperation = 1
	OperationDelete RelationshipOperation = 2
)

// WriteRelationshipsResponse is the result of WriteRelationships.
type WriteRelationshipsResponse struct {
	WrittenAt string
}

// WriteRelationships writes or deletes relationships.
func (c *Client) WriteRelationships(ctx context.Context, req WriteRelationshipsRequest) (*WriteRelationshipsResponse, error) {
	ctx, cancel := c.ctx(ctx)
	defer cancel()

	updates := make([]*pb.RelationshipUpdate, len(req.Updates))
	for i, u := range req.Updates {
		updates[i] = &pb.RelationshipUpdate{
			Operation: pb.RelationshipUpdate_Operation(u.Operation),
			Relationship: &pb.Relationship{
				Object:   objectRefToProto(u.Object),
				Relation: u.Relation,
				Subject:  subjectRefToProto(u.Subject),
			},
		}
	}

	resp, err := c.relationships.WriteRelationships(ctx, &pb.WriteRelationshipsRequest{
		Updates: updates,
	})
	if err != nil {
		return nil, errorFromStatus(err)
	}

	var writtenAt string
	if resp.WrittenAt != nil {
		writtenAt = resp.WrittenAt.Token
	}

	return &WriteRelationshipsResponse{WrittenAt: writtenAt}, nil
}

// Relationship represents a stored relationship tuple.
type Relationship struct {
	Object   ObjectRef
	Relation string
	Subject  SubjectRef
}

// ReadRelationshipsRequest is the input for ReadRelationships.
type ReadRelationshipsRequest struct {
	ObjectType string
	ObjectID   string
	Relation   string
}

// ReadRelationships reads relationships matching the given filter.
func (c *Client) ReadRelationships(ctx context.Context, req ReadRelationshipsRequest) ([]Relationship, error) {
	ctx, cancel := c.ctx(ctx)
	defer cancel()

	filter := &pb.RelationshipFilter{}
	if req.ObjectType != "" {
		filter.ObjectType = &req.ObjectType
	}
	if req.ObjectID != "" {
		filter.ObjectId = &req.ObjectID
	}
	if req.Relation != "" {
		filter.Relation = &req.Relation
	}

	resp, err := c.relationships.ReadRelationships(ctx, &pb.ReadRelationshipsRequest{
		Consistency: fullConsistency(),
		Filter:      filter,
	})
	if err != nil {
		return nil, errorFromStatus(err)
	}

	var results []Relationship
	for _, rel := range resp.Relationships {
		if rel.Object == nil || rel.Subject == nil {
			continue
		}
		results = append(results, Relationship{
			Object:   protoToObjectRef(rel.Object),
			Relation: rel.Relation,
			Subject:  protoToSubjectRef(rel.Subject),
		})
	}

	return results, nil
}

// LookupResourcesRequest is the input for LookupResources.
type LookupResourcesRequest struct {
	ResourceType string
	Permission   string
	Subject      SubjectRef
}

// LookupResources finds all resources of a given type the subject has permission on.
func (c *Client) LookupResources(ctx context.Context, req LookupResourcesRequest) ([]string, error) {
	ctx, cancel := c.ctx(ctx)
	defer cancel()

	stream, err := c.permissions.LookupResources(ctx, &pb.LookupResourcesRequest{
		Consistency:  fullConsistency(),
		ResourceType: req.ResourceType,
		Permission:   req.Permission,
		Subject:      subjectRefToProto(req.Subject),
	})
	if err != nil {
		return nil, errorFromStatus(err)
	}

	var ids []string
	for {
		resp, err := stream.Recv()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, errorFromStatus(err)
		}
		ids = append(ids, resp.ResourceId)
	}

	return ids, nil
}

// LookupSubjectsRequest is the input for LookupSubjects.
type LookupSubjectsRequest struct {
	Resource    ObjectRef
	Permission  string
	SubjectType string
}

// LookupSubjects finds all subjects of a given type with a permission on a resource.
func (c *Client) LookupSubjects(ctx context.Context, req LookupSubjectsRequest) ([]SubjectRef, error) {
	ctx, cancel := c.ctx(ctx)
	defer cancel()

	stream, err := c.permissions.LookupSubjects(ctx, &pb.LookupSubjectsRequest{
		Consistency: fullConsistency(),
		Resource:    objectRefToProto(req.Resource),
		Permission:  req.Permission,
		SubjectType: req.SubjectType,
	})
	if err != nil {
		return nil, errorFromStatus(err)
	}

	var subjects []SubjectRef
	for {
		resp, err := stream.Recv()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, errorFromStatus(err)
		}
		if resp.Subject != nil {
			subjects = append(subjects, protoToSubjectRef(resp.Subject))
		}
	}

	return subjects, nil
}

// WriteSchema writes the authorization schema to the server.
func (c *Client) WriteSchema(ctx context.Context, schema string, force bool) (*WriteSchemaResponse, error) {
	ctx, cancel := c.ctx(ctx)
	defer cancel()

	resp, err := c.schemas.WriteSchema(ctx, &pb.WriteSchemaRequest{
		Schema: schema,
		Force:  force,
	})
	if err != nil {
		return nil, errorFromStatus(err)
	}

	return &WriteSchemaResponse{
		BreakingChangesOverridden: resp.BreakingChangesOverridden,
	}, nil
}

// WriteSchemaResponse is the result of WriteSchema.
type WriteSchemaResponse struct {
	BreakingChangesOverridden bool
}

// ReadSchema reads the current authorization schema from the server.
func (c *Client) ReadSchema(ctx context.Context) (string, error) {
	ctx, cancel := c.ctx(ctx)
	defer cancel()

	resp, err := c.schemas.ReadSchema(ctx, &pb.ReadSchemaRequest{})
	if err != nil {
		return "", errorFromStatus(err)
	}

	return resp.Schema, nil
}

// Helpers

func objectRefToProto(o ObjectRef) *pb.ObjectReference {
	return &pb.ObjectReference{
		ObjectType: o.ObjectType,
		ObjectId:   o.ObjectID,
	}
}

func subjectRefToProto(s SubjectRef) *pb.SubjectReference {
	ref := &pb.SubjectReference{
		SubjectType: s.SubjectType,
		SubjectId:   s.SubjectID,
	}
	if s.SubjectRelation != "" {
		ref.SubjectRelation = &s.SubjectRelation
	}
	return ref
}

func protoToObjectRef(o *pb.ObjectReference) ObjectRef {
	return ObjectRef{
		ObjectType: o.ObjectType,
		ObjectID:   o.ObjectId,
	}
}

func protoToSubjectRef(s *pb.SubjectReference) SubjectRef {
	rel := ""
	if s.SubjectRelation != nil {
		rel = *s.SubjectRelation
	}
	return SubjectRef{
		SubjectType:     s.SubjectType,
		SubjectID:       s.SubjectId,
		SubjectRelation: rel,
	}
}

func fullConsistency() *pb.Consistency {
	return &pb.Consistency{
		Requirement: &pb.Consistency_FullConsistency{
			FullConsistency: true,
		},
	}
}

func apiKeyInterceptor(key string) grpc.UnaryClientInterceptor {
	return func(ctx context.Context, method string, req, reply any, cc *grpc.ClientConn, invoker grpc.UnaryInvoker, opts ...grpc.CallOption) error {
		ctx = metadata.AppendToOutgoingContext(ctx, "authorization", "Bearer "+key)
		return invoker(ctx, method, req, reply, cc, opts...)
	}
}

func apiKeyStreamInterceptor(key string) grpc.StreamClientInterceptor {
	return func(ctx context.Context, desc *grpc.StreamDesc, cc *grpc.ClientConn, method string, streamer grpc.Streamer, opts ...grpc.CallOption) (grpc.ClientStream, error) {
		ctx = metadata.AppendToOutgoingContext(ctx, "authorization", "Bearer "+key)
		return streamer(ctx, desc, cc, method, opts...)
	}
}
