import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { Genome, Mesh } from "./genome";

const GEO = {
  sphere: new THREE.SphereGeometry(1, 32, 24),
  cone: new THREE.ConeGeometry(1, 1, 16),
  torus: new THREE.TorusGeometry(1, 0.08, 10, 40),
  octa: new THREE.OctahedronGeometry(1, 0),
  ico: new THREE.IcosahedronGeometry(1, 1),
  box: new THREE.BoxGeometry(1, 1, 1),
  cylinder: new THREE.CylinderGeometry(1, 1, 1, 16),
};

function Part({ m, genome }: { m: Mesh; genome: Genome }) {
  const ref = useRef<THREE.Mesh>(null);
  const color = genome.palette[m.role];
  const geometry = useMemo(() => {
    if (m.kind !== "tube" || !m.points) return GEO[m.kind as keyof typeof GEO] ?? GEO.sphere;
    const curve = new THREE.CatmullRomCurve3(m.points.map((p) => new THREE.Vector3(...p)));
    return new THREE.TubeGeometry(curve, 24, m.radius ?? 0.05, 8, false);
  }, [m]);
  useEffect(() => () => {
    if (m.kind === "tube") geometry.dispose();
  }, [geometry, m.kind]);
  const emissive = !genome.revoked && m.emissive ? color : "#000000";
  useFrame(({ clock }) => {
    if (!ref.current || m.pulse === undefined || genome.revoked) return;
    const mat = ref.current.material as THREE.MeshStandardMaterial;
    mat.emissiveIntensity = (m.emissive ?? 1) * (0.75 + 0.35 * Math.sin(clock.elapsedTime * 2.2 + m.pulse));
  });
  return (
    <mesh ref={ref} geometry={geometry} position={m.kind === "tube" ? [0, 0, 0] : m.pos} scale={m.kind === "tube" ? [1, 1, 1] : m.scale}
          rotation={m.rot ?? [0, 0, 0]} castShadow name={m.tag}>
      <meshStandardMaterial color={color} emissive={emissive} emissiveIntensity={m.emissive ?? 0}
                            roughness={m.role === "metal" ? 0.25 : m.role === "eye" ? 0.15 : 0.55}
                            metalness={m.role === "metal" ? 0.85 : m.role === "plate" ? 0.3 : 0.05}
                            wireframe={!!m.wire} transparent={m.opacity !== undefined} opacity={m.opacity ?? 1}
                            depthWrite={m.opacity === undefined} />
    </mesh>
  );
}

function Body({ genome, groupRef }: { genome: Genome; groupRef: React.RefObject<THREE.Group | null> }) {
  useFrame(({ clock }) => {
    const g = groupRef.current;
    if (!g) return;
    const t = clock.elapsedTime;
    if (genome.revoked) {
      g.rotation.set(0.05, 0.4, -0.22);
      g.position.set(0, -0.15, 0);
      return;
    }
    if (genome.idle === "sway") {
      g.rotation.y = 0.35 * Math.sin(t * 0.6);
      g.position.y = 0;
    } else if (genome.idle === "hover") {
      g.position.y = 0.18 + 0.12 * Math.sin(t * 1.3);
      g.rotation.y = t * 0.25;
    } else {
      const s = 1 + 0.018 * Math.sin(t * 1.6);
      g.scale.set(s, 1 + 0.028 * Math.sin(t * 1.6), s);
      g.rotation.y = 0.25 * Math.sin(t * 0.25);
    }
  });
  return (
    <group ref={groupRef}>
      {genome.meshes.map((m, i) => <Part key={`${m.tag}-${i}`} m={m} genome={genome} />)}
    </group>
  );
}

function Controls() {
  const { camera, gl } = useThree();
  useEffect(() => {
    const c = new OrbitControls(camera, gl.domElement);
    c.enablePan = false;
    c.minDistance = 3;
    c.maxDistance = 11;
    c.target.set(0, 0.25, 0);
    c.update();
    return () => c.dispose();
  }, [camera, gl]);
  return null;
}

export interface CreatureHandle { canvas: HTMLCanvasElement | null; group: THREE.Group | null }

export function CreatureView({ genome, className, handle, interactive = true }: {
  genome: Genome; className?: string; handle?: React.RefObject<CreatureHandle>; interactive?: boolean;
}) {
  const groupRef = useRef<THREE.Group>(null);
  return (
    <div className={className}>
      <Canvas shadows camera={{ position: [3.2, 1.6, 5.2], fov: 42 }} gl={{ preserveDrawingBuffer: true, antialias: true }}
              onCreated={({ gl, scene }) => {
                gl.toneMapping = THREE.ACESFilmicToneMapping;
                scene.fog = new THREE.Fog("#05070b", 7, 16);
                if (handle) handle.current = { canvas: gl.domElement, group: groupRef.current };
              }}>
        <color attach="background" args={[genome.revoked ? "#0b0b0d" : "#05070b"]} />
        <ambientLight intensity={genome.revoked ? 0.25 : 0.35} />
        <hemisphereLight args={[genome.revoked ? "#777" : genome.palette.glow, "#1b1030", genome.revoked ? 0.2 : 0.45]} />
        <directionalLight position={[4, 6, 5]} intensity={genome.revoked ? 0.6 : 1.4} castShadow />
        <pointLight position={[-3, 1.5, 2]} intensity={genome.revoked ? 0 : 6} distance={10} color={genome.palette.glow} />
        <Body genome={genome} groupRef={groupRef} />
        <mesh rotation={[-Math.PI / 2, 0, 0]} position={[0, -1.3, 0]} receiveShadow>
          <circleGeometry args={[4, 48]} />
          <meshStandardMaterial color="#0b1018" roughness={0.9} />
        </mesh>
        <mesh rotation={[-Math.PI / 2, 0, 0]} position={[0, -1.29, 0]}>
          <ringGeometry args={[2.2, 2.26, 64]} />
          <meshBasicMaterial color={genome.revoked ? "#333" : genome.palette.glow} transparent opacity={0.35} />
        </mesh>
        {interactive && <Controls />}
        <HandleSync handle={handle} groupRef={groupRef} />
      </Canvas>
    </div>
  );
}

function HandleSync({ handle, groupRef }: { handle?: React.RefObject<CreatureHandle>; groupRef: React.RefObject<THREE.Group | null> }) {
  const { gl } = useThree();
  useEffect(() => {
    if (handle) handle.current = { canvas: gl.domElement, group: groupRef.current };
  });
  return null;
}
