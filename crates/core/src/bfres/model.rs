#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub name: String,
    pub models: Vec<Model>,
    pub skeletal_anims: Vec<SkeletalAnim>,
    pub material_anims: Vec<MaterialAnim>,
    pub bone_vis_anims: Vec<BoneVisAnim>,
    pub scene_anims: Vec<SceneAnim>,
    pub external: Vec<(String, Vec<u8>)>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub name: String,
    pub path: String,
    pub skeleton: Skeleton,
    pub vertex_buffers: Vec<VertexBuffer>,
    pub shapes: Vec<Shape>,
    pub materials: Vec<Material>,
    pub user_data: Vec<UserData>,
    pub total_vertices: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    pub flags: u32,
    pub bones: Vec<Bone>,
    pub smooth_count: u16,
    pub rigid_count: u16,
    pub matrix_to_bone: Vec<u16>,
    pub inverse: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    pub index: u16,
    pub parent: u16,
    pub smooth: i16,
    pub rigid: i16,
    pub billboard: i16,
    pub flags: u32,
    pub srt: [f32; 10],
    pub user_data: Vec<UserData>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VertexBuffer {
    pub vertex_count: u32,
    pub skin_count: u8,
    pub attributes: Vec<Attribute>,
    pub buffers: Vec<Buffer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub buffer: u8,
    pub offset: u16,
    pub format: u32,
    pub gx2_format: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Buffer {
    pub stride: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub name: String,
    pub flags: u32,
    pub index: u16,
    pub material: u16,
    pub bone: u16,
    pub vertex_buffer: u16,
    pub skin_bones: Vec<u16>,
    pub vertex_skin_count: u8,
    pub target_attributes: u8,
    pub meshes: Vec<Mesh>,
    pub bounding_boxes: Vec<f32>,
    pub radii: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub primitive: u32,
    pub index_format: u32,
    pub index_count: u32,
    pub first_vertex: u32,
    pub sub_meshes: Vec<(u32, u32)>,
    pub indices: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenderValue {
    Int(Vec<i32>),
    Float(Vec<f32>),
    Str(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderInfo {
    pub name: String,
    pub value: RenderValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShaderAssign {
    pub archive: String,
    pub model: String,
    pub revision: u32,
    pub attributes: Vec<(String, String)>,
    pub samplers: Vec<(String, String)>,
    pub options: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShaderParam {
    pub name: String,
    pub kind: u8,
    pub size: u8,
    pub offset: u16,
    pub unknown: i32,
    pub depended: u16,
    pub depend: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    pub flags: u32,
    pub render_infos: Vec<RenderInfo>,
    pub shader_assign: ShaderAssign,
    pub textures: Vec<String>,
    pub samplers: Vec<(String, [u8; 0x20])>,
    pub params: Vec<ShaderParam>,
    pub param_data: Vec<u8>,
    pub volatile_count: u16,
    pub volatile_flags: Vec<u8>,
    pub raw_param_size: u16,
    pub user_data: Vec<UserData>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UserValue {
    Int(Vec<i32>),
    Float(Vec<f32>),
    Str(Vec<String>, bool),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserData {
    pub name: String,
    pub value: UserValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterialAnim {
    pub name: String,
    pub path: String,
    pub flags: u32,
    pub frames: i32,
    pub baked_size: u32,
    pub bind_indices: Vec<u16>,
    pub param_anim_count: u16,
    pub pattern_anim_count: u16,
    pub textures: Vec<String>,
    pub materials: Vec<MaterialAnimData>,
    pub user_data: Vec<UserData>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterialAnimData {
    pub name: String,
    pub param_curve_begin: i16,
    pub pattern_curve_begin: i16,
    pub visibility: [i16; 3],
    pub params: Vec<ParamAnimInfo>,
    pub patterns: Vec<PatternAnimInfo>,
    pub curves: Vec<Curve>,
    pub constants: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamAnimInfo {
    pub name: String,
    pub begin_curve: u16,
    pub float_curves: u16,
    pub int_curves: u16,
    pub begin_constant: u16,
    pub constants: u16,
    pub sub_bind: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternAnimInfo {
    pub name: String,
    pub curve: i16,
    pub begin_constant: i16,
    pub sub_bind: i8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub flags: u16,
    pub key_count: u16,
    pub target: u32,
    pub start: f32,
    pub end: f32,
    pub scale: u32,
    pub offset: u32,
    pub delta: f32,
    pub frames: Vec<u8>,
    pub keys: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkeletalAnim {
    pub name: String,
    pub path: String,
    pub flags: u32,
    pub frames: i32,
    pub baked_size: u32,
    pub bind_indices: Vec<u16>,
    pub bones: Vec<BoneAnim>,
    pub user_data: Vec<UserData>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoneAnim {
    pub name: String,
    pub flags: u32,
    pub begin_rotate: u8,
    pub begin_translate: u8,
    pub base_translate: u8,
    pub begin_curve: i32,
    pub curves: Vec<Curve>,
    pub base: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoneVisAnim {
    pub name: String,
    pub path: String,
    pub flags: u32,
    pub frames: i32,
    pub baked_size: u32,
    pub bind_indices: Vec<u16>,
    pub names: Vec<String>,
    pub curves: Vec<Curve>,
    pub base: Vec<u32>,
    pub user_data: Vec<UserData>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneAnim {
    pub name: String,
    pub path: String,
    pub cameras: Vec<CameraAnim>,
    pub user_data: Vec<UserData>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraAnim {
    pub name: String,
    pub flags: u16,
    pub frames: i32,
    pub baked_size: u32,
    pub curves: Vec<Curve>,
    pub base: Vec<f32>,
    pub user_data: Vec<UserData>,
}
